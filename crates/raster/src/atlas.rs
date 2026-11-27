//! Glyph atlases. Pages of `R8Unorm` coverage for ordinary antialiased
//! text and `Rgba8UnormSrgb` for color glyphs (emoji, COLR, bitmap strikes).
//!
//! The raster queue packs bitmaps in draw order. Workers may prepare missing
//! bitmaps ahead of replay. A glyph's key covers every input
//! that changes its bitmap: face, glyph ID, physical size, subpixel
//! position bucket, variation coordinates, and synthetic bold or skew.
//! Entries are shelf-packed. Under pressure, reclaim cold buckets on a page
//! that no draw has referenced in the current transaction.
//!
//! Blank glyphs (whitespace, missing outlines) cache an empty entry.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use engine::scene::FontFaceId;
use etagere::BucketedAtlasAllocator;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};

use crate::compiler::GlyphDraw;
use crate::fonts::FontLibrary;
use crate::gpu::Gpu;
use crate::prepare::Background;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};

mod pages;
mod rasterize;

use rasterize::{Bitmap, GlyphRasterizer};

/// Quarter-pixel horizontal positions.
pub(crate) const SUBPIXEL_BUCKETS: u32 = 4;

#[derive(Clone, Copy)]
pub(crate) struct SubpixelPosition {
    pub point: [f32; 2],
    pub buckets: u32,
}
const PAGE_SIZE: u32 = 2048;
/// Upper bound on simultaneously resident pages on small adapters.
const PAGE_LIMIT: u32 = 6;
/// One-texel gutter between glyphs so bilinear sampling never bleeds.
const GUTTER: u32 = 1;
const PREPARED_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const PREPARED_MASK_LIMIT: usize = 256;

/// Identity of one rasterized glyph bitmap.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    face: Arc<FontFaceId>,
    glyph: u16,
    variant: GlyphVariant,
    coords: Option<Arc<[i16]>>,
    embolden: bool,
    skew: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphVariant {
    /// Physical font size in 1/64 pixel steps.
    size: u32,
    /// Horizontal subpixel bucket, `0..SUBPIXEL_BUCKETS`.
    subpixel: u32,
    darken_stem: bool,
    hint: bool,
}

impl GlyphKey {
    fn new(draw: &GlyphDraw, variant: GlyphVariant) -> Self {
        Self {
            face: Arc::clone(&draw.face),
            glyph: draw.glyph,
            variant,
            coords: draw.coords.clone(),
            embolden: draw.embolden,
            skew: draw.skew.map(f32::to_bits),
        }
    }
}

/// Each compiled glyph remembers the last slot it used on its Raster device.
/// The generational key prevents reused slots from aliasing evicted entries.
#[derive(Debug, Default)]
pub(crate) struct GlyphSlot(AtomicU64);

impl GlyphSlot {
    fn get(&self) -> DefaultKey {
        KeyData::from_ffi(self.0.load(Ordering::Relaxed)).into()
    }

    fn set(&self, slot: DefaultKey) {
        self.0.store(slot.data().as_ffi(), Ordering::Relaxed);
    }
}

impl Clone for GlyphSlot {
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

impl PartialEq for GlyphSlot {
    fn eq(&self, _other: &Self) -> bool {
        // The hint is not part of the compiled picture's visual identity.
        true
    }
}

/// Texel rectangle of a glyph in one atlas page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    allocation: Option<etagere::AllocId>,
}

/// Where a glyph's bitmap landed, for the encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub rect: AtlasRect,
    /// Bitmap left edge relative to the baseline, in pixels.
    pub left: i32,
    /// Bitmap top edge relative to the baseline, in pixels. Negative is up.
    pub top: i32,
    pub color: bool,
    page_generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    variant: GlyphVariant,
    placement: Placement,
    used: u64,
}

const TILE_SIDE: u32 = 128;

struct AtlasTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    layers: u32,
}

impl AtlasTexture {
    fn new(gpu: &Gpu, color: bool, size: [u32; 2], layers: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto glyph atlas"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if color {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::R8Unorm
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Self {
            texture,
            view,
            layers,
        }
    }
}

#[derive(Default)]
struct AtlasArray {
    texture: Option<AtlasTexture>,
    retired: Vec<AtlasTexture>,
    pages: Vec<Page>,
}

struct Page {
    alloc: BucketedAtlasAllocator,
    size: [u32; 2],
    backing: PageBacking,
    age: u64,
    generation: u64,
}

enum PageBacking {
    Resident {
        layer: u32,
        tiles: AtlasTiles,
    },
    Temporary {
        texture: AtlasTexture,
        material: Option<wgpu::BindGroup>,
    },
}

struct AtlasTiles {
    columns: u32,
    tiles: Box<[Option<AtlasTile>]>,
}

struct AtlasTile {
    pixels: Box<[u8]>,
    dirty: Option<[u32; 4]>,
}

impl AtlasTiles {
    fn new([width, height]: [u32; 2]) -> Self {
        let columns = width.div_ceil(TILE_SIDE);
        let rows = height.div_ceil(TILE_SIDE);
        Self {
            columns,
            tiles: std::iter::repeat_with(|| None)
                .take((columns * rows) as usize)
                .collect(),
        }
    }

    fn write(&mut self, [x, y]: [u32; 2], [width, height]: [u32; 2], channels: u32, data: &[u8]) {
        let right = x + width;
        let bottom = y + height;
        let stride = TILE_SIDE * channels;
        for ty in y / TILE_SIDE..=(bottom - 1) / TILE_SIDE {
            for tx in x / TILE_SIDE..=(right - 1) / TILE_SIDE {
                let origin_x = tx * TILE_SIDE;
                let origin_y = ty * TILE_SIDE;
                let x0 = x.max(origin_x);
                let y0 = y.max(origin_y);
                let x1 = right.min(origin_x + TILE_SIDE);
                let y1 = bottom.min(origin_y + TILE_SIDE);
                let tile = self.tiles[(ty * self.columns + tx) as usize].get_or_insert_with(|| {
                    AtlasTile {
                        pixels: vec![0; (stride * TILE_SIDE) as usize].into_boxed_slice(),
                        dirty: None,
                    }
                });
                let bytes = ((x1 - x0) * channels) as usize;
                for row in y0..y1 {
                    let source = (((row - y) * width + (x0 - x)) * channels) as usize;
                    let target = ((row - origin_y) * stride + (x0 - origin_x) * channels) as usize;
                    tile.pixels[target..target + bytes]
                        .copy_from_slice(&data[source..source + bytes]);
                }
                let patch = [x0 - origin_x, y0 - origin_y, x1 - origin_x, y1 - origin_y];
                tile.dirty = Some(tile.dirty.map_or(patch, |[left, top, right, bottom]| {
                    [
                        left.min(patch[0]),
                        top.min(patch[1]),
                        right.max(patch[2]),
                        bottom.max(patch[3]),
                    ]
                }));
            }
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AtlasBinding {
    #[default]
    Resident,
    Temporary {
        color: bool,
        page: usize,
    },
}

/// Rasterizer for glyph bitmaps and the pages that hold them.
pub struct Atlas {
    rasterizer: GlyphRasterizer,
    prepared: HashMap<GlyphKey, Bitmap>,
    prepared_bytes: usize,
    preparation_owner: Option<u64>,
    pending_masks: Background<Vec<(GlyphKey, Bitmap)>>,
    deferred: HashSet<GlyphKey>,
    deadline: Option<std::time::Instant>,
    mask: AtlasArray,
    color: AtlasArray,
    entries: HashMap<GlyphKey, DefaultKey>,
    slots: SlotMap<DefaultKey, Entry>,
    clock: u64,
    page_size: u32,
    mask_channels: u32,
    epoch: u64,
    blank: AtlasTexture,
    material: Option<wgpu::BindGroup>,
    stats: AtlasStats,
    scratch: Vec<u8>,
    darken_small: bool,
    pinned: HashSet<(bool, usize)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    pub hits: u32,
    pub misses: u32,
    pub uploads: u32,
    pub upload_calls: u32,
    pub upload_bytes: u64,
    pub copy_pixels: u64,
    pub evictions: u32,
    pub region_reclaims: u32,
    pub prerasterized: u32,
    /// CPU time spent producing missing masks while drawing.
    pub mask_raster: std::time::Duration,
    /// Live entries across both atlases.
    pub entries: usize,
    /// Bytes of texture memory in live pages.
    pub bytes: usize,
}

impl Atlas {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            rasterizer: GlyphRasterizer::default(),
            prepared: HashMap::new(),
            prepared_bytes: 0,
            preparation_owner: None,
            pending_masks: Background::default(),
            deferred: Default::default(),
            deadline: None,
            mask: AtlasArray {
                texture: Some(AtlasTexture::new(
                    gpu,
                    false,
                    [gpu.max_texture_size().min(PAGE_SIZE); 2],
                    1,
                )),
                ..AtlasArray::default()
            },
            color: AtlasArray::default(),
            entries: HashMap::new(),
            slots: SlotMap::new(),
            clock: 0,
            page_size: gpu.max_texture_size().min(PAGE_SIZE),
            mask_channels: 1,
            epoch: 0,
            blank: AtlasTexture::new(gpu, true, [1, 1], 1),
            material: None,
            stats: AtlasStats::default(),
            scratch: Vec::new(),
            darken_small: true,
            pinned: HashSet::new(),
        }
    }

    pub fn trim(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.mask = AtlasArray::default();
        self.color = AtlasArray::default();
        self.material = None;
        self.entries.clear();
        self.slots.clear();
        self.rasterizer.clear_font_keys();
        self.set_prepare_owner(None);
        self.scratch = Vec::new();
        self.pinned.clear();
        self.stats = AtlasStats::default();
    }

    /// Starts a transaction after the previous frame was submitted or discarded.
    pub fn begin_frame(&mut self) {
        self.pinned.clear();
        self.deferred.clear();
        self.collect_prepared();
        let has_temporary = self
            .mask
            .pages
            .iter()
            .chain(&self.color.pages)
            .any(Page::temporary);
        let before = self.entries.len();
        if has_temporary {
            self.epoch = self.epoch.wrapping_add(1);
            let masks = retain_pages(&mut self.mask.pages);
            let colors = retain_pages(&mut self.color.pages);
            self.slots.retain(|_, entry| {
                let rect = &mut entry.placement.rect;
                if rect.page == usize::MAX {
                    return true;
                }
                let pages = if entry.placement.color {
                    &colors
                } else {
                    &masks
                };
                let Some(index) = pages.get(rect.page).copied().flatten() else {
                    return false;
                };
                rect.page = index;
                true
            });
            self.entries
                .retain(|_, slot| self.slots.contains_key(*slot));
        }
        self.stats.evictions = u32::try_from(before - self.entries.len()).unwrap_or(u32::MAX);
        if self.entries.len() > 32768 {
            self.slots
                .retain(|_, entry| entry.placement.rect.page != usize::MAX);
            self.entries
                .retain(|_, slot| self.slots.contains_key(*slot));
        }
        self.stats.bytes = self.mask.bytes(self.mask_channels as usize) + self.color.bytes(4);
        self.stats.entries = self.entries.len();
    }

    pub fn texels(&self, placement: Placement) -> Option<[u16; 4]> {
        let rect = placement.rect;
        Some([
            u16::try_from(rect.x).ok()?,
            u16::try_from(rect.y).ok()?,
            u16::try_from(rect.x + rect.w).ok()?,
            u16::try_from(rect.y + rect.h).ok()?,
        ])
    }

    pub fn inverse_page_size(&self) -> f32 {
        1.0 / self.page_size as f32
    }

    pub fn pin(&mut self, placement: Placement) {
        if placement.rect.page != usize::MAX {
            self.pinned.insert((placement.color, placement.rect.page));
            let clock = self.clock;
            if let Some(page) = self.pages_for(placement.color).get_mut(placement.rect.page) {
                page.age = clock;
            }
        }
    }

    /// Counters since the last take.
    pub fn take_stats(&mut self) -> AtlasStats {
        let stats = self.stats;
        self.stats = AtlasStats {
            entries: stats.entries,
            bytes: stats.bytes,
            ..AtlasStats::default()
        };
        stats
    }

    /// Looks up a glyph, rasterizing and uploading it on a miss.
    ///
    /// `size` is the physical font size in pixels. `(sx, sy)` is the glyph's
    /// fractional baseline position in pixels, used to pick a subpixel
    /// bitmap so the edge lands where the layout put it. `None` when the
    /// face is gone, the size is out of range, or the glyph is too large.
    pub fn glyph(
        &mut self,
        gpu: &Gpu,
        library: &FontLibrary,
        draw: &GlyphDraw,
        size: f32,
        position: Option<SubpixelPosition>,
    ) -> Option<Placement> {
        let variant = self.variant(gpu, size, position)?;
        self.clock = self.clock.wrapping_add(1);
        let slot = draw.atlas_slot.get();
        if self
            .slots
            .get(slot)
            .is_some_and(|entry| entry.variant == variant)
        {
            return self.hit(slot);
        }
        let key = GlyphKey::new(draw, variant);
        if let Some(&slot) = self.entries.get(&key) {
            draw.atlas_slot.set(slot);
            return self.hit(slot);
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let bitmap = self
            .prepared
            .remove(&key)
            .inspect(|bitmap| {
                self.prepared_bytes -= bitmap.bytes();
            })
            .or_else(|| {
                let started = std::time::Instant::now();
                if self.deadline.is_some_and(|deadline| started >= deadline) {
                    if library.snapshot(&key.face).is_some() {
                        self.deferred.insert(key.clone());
                    }
                    return None;
                }
                let bitmap = self.rasterizer.glyph(library, &key);
                self.stats.mask_raster += started.elapsed();
                bitmap
            })?;
        let (slot, placement) = self.store(gpu, key, bitmap)?;
        draw.atlas_slot.set(slot);
        if placement.rect.page != usize::MAX {
            self.stats.uploads = self.stats.uploads.saturating_add(1);
        }
        self.pin(placement);
        Some(placement)
    }

    fn hit(&mut self, slot: DefaultKey) -> Option<Placement> {
        let entry = self.slots.get_mut(slot)?;
        entry.used = self.clock;
        self.stats.hits = self.stats.hits.saturating_add(1);
        let placement = entry.placement;
        self.pin(placement);
        Some(placement)
    }

    fn variant(
        &self,
        gpu: &Gpu,
        size: f32,
        position: Option<SubpixelPosition>,
    ) -> Option<GlyphVariant> {
        let size = quantize_size(size, gpu.max_texture_size(), position.is_some())?;
        Some(GlyphVariant {
            size,
            subpixel: position.map_or(0, |pos| horizontal_bucket(pos.point[0], pos.buckets)),
            darken_stem: self.darken_small && position.is_some() && size <= 14 * 64,
            hint: position.is_some(),
        })
    }

    pub(crate) fn missing_glyph(
        &self,
        gpu: &Gpu,
        draw: &GlyphDraw,
        size: f32,
        position: Option<SubpixelPosition>,
    ) -> Option<GlyphKey> {
        let variant = self.variant(gpu, size, position)?;
        if self
            .slots
            .get(draw.atlas_slot.get())
            .is_some_and(|entry| entry.variant == variant)
        {
            return None;
        }
        let key = GlyphKey::new(draw, variant);
        (!self.entries.contains_key(&key) && !self.prepared.contains_key(&key)).then_some(key)
    }

    pub(crate) fn prepare(
        &mut self,
        library: &FontLibrary,
        keys: Vec<GlyphKey>,
        deadline: std::time::Instant,
        notify: Option<&Arc<dyn Fn() + Send + Sync>>,
    ) {
        let mut keys = keys.into_iter();
        if let Some(notify) = notify {
            let parallel = keys.len().saturating_sub(32);
            if parallel >= 8 {
                self.prepare_background(library, keys.by_ref().take(parallel), notify);
            }
        }
        while std::time::Instant::now() < deadline
            && let Some(key) = keys.next()
        {
            if let Some(bitmap) = self.rasterizer.glyph(library, &key) {
                self.cache_prepared(key, bitmap);
            }
        }
        if let Some(notify) = notify {
            self.prepare_background(library, keys, notify);
        }
    }

    pub(crate) fn set_deadline(&mut self, deadline: Option<std::time::Instant>) {
        self.deadline = deadline;
    }

    pub(crate) fn defer_remaining(
        &mut self,
        library: &FontLibrary,
        notify: &Arc<dyn Fn() + Send + Sync>,
    ) -> bool {
        if self.deferred.is_empty() {
            return false;
        }
        let keys = std::mem::take(&mut self.deferred);
        self.prepare_background(library, keys.into_iter(), notify);
        true
    }

    fn prepare_background(
        &mut self,
        library: &FontLibrary,
        keys: impl Iterator<Item = GlyphKey>,
        notify: &Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.pending_masks.busy() {
            return;
        }
        let available = PREPARED_MASK_LIMIT
            .saturating_sub(self.prepared.len())
            .max(1);
        let mut estimate = 0usize;
        let snapshots: Vec<_> = keys
            .filter_map(|key| Some((library.snapshot(&key.face)?, key)))
            .take(available)
            .take_while(|(_, key)| {
                let ppem = key.variant.size.div_ceil(64) as usize;
                let size = ppem.saturating_mul(ppem).saturating_mul(4);
                let previous = estimate;
                estimate = estimate.saturating_add(size);
                previous == 0 || estimate <= PREPARED_BYTES
            })
            .collect();
        if snapshots.is_empty() {
            return;
        }
        self.pending_masks
            .offer(Arc::clone(notify), move |cancelled| {
                snapshots
                    .into_par_iter()
                    .map_init(
                        GlyphRasterizer::default,
                        |rasterizer, ((font, index), key)| {
                            if cancelled() {
                                return None;
                            }
                            Some((
                                key.clone(),
                                rasterizer
                                    .rasterize(font.data(), index, &key)
                                    .unwrap_or(Bitmap::Empty),
                            ))
                        },
                    )
                    .flatten()
                    .collect()
            });
    }

    fn cache_prepared(&mut self, key: GlyphKey, bitmap: Bitmap) {
        let bytes = bitmap.bytes();
        if self.prepared.len() < PREPARED_MASK_LIMIT
            && (self.prepared.is_empty() || self.prepared_bytes + bytes <= PREPARED_BYTES)
            && !self.entries.contains_key(&key)
            && let std::collections::hash_map::Entry::Vacant(entry) = self.prepared.entry(key)
        {
            entry.insert(bitmap);
            self.prepared_bytes += bytes;
            self.stats.prerasterized = self.stats.prerasterized.saturating_add(1);
        }
    }

    pub(crate) fn collect_prepared(&mut self) -> bool {
        let Some(ready) = self.pending_masks.take_ready() else {
            return false;
        };
        let had_glyphs = !ready.is_empty();
        for (key, bitmap) in ready {
            self.cache_prepared(key, bitmap);
        }
        had_glyphs
    }

    pub(crate) fn set_prepare_owner(&mut self, owner: Option<u64>) {
        if self.preparation_owner == owner && owner.is_some() {
            return;
        }
        self.preparation_owner = owner;
        self.pending_masks.cancel();
        self.prepared.clear();
        self.prepared_bytes = 0;
    }

    fn store(
        &mut self,
        gpu: &Gpu,
        key: GlyphKey,
        bitmap: Bitmap,
    ) -> Option<(DefaultKey, Placement)> {
        let placement = match bitmap {
            Bitmap::Empty => Placement {
                rect: AtlasRect {
                    page: usize::MAX,
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                    allocation: None,
                },
                left: 0,
                top: 0,
                color: false,
                page_generation: 0,
            },
            Bitmap::Mask(image) => self.store_image(gpu, false, image)?,
            Bitmap::Color(image) => self.store_image(gpu, true, image)?,
        };
        let slot = self.insert(key, placement, self.clock);
        self.stats.entries = self.entries.len();
        Some((slot, placement))
    }

    fn insert(&mut self, key: GlyphKey, placement: Placement, used: u64) -> DefaultKey {
        let slot = self.slots.insert(Entry {
            variant: key.variant,
            placement,
            used,
        });
        if let Some(old) = self.entries.insert(key, slot) {
            self.slots.remove(old);
        }
        slot
    }

    fn store_image(
        &mut self,
        gpu: &Gpu,
        color: bool,
        image: swash::scale::image::Image,
    ) -> Option<Placement> {
        {
            let (w, h) = (image.placement.width, image.placement.height);
            let rect = self.allocate(gpu, color, w, h)?;
            if w > 0 && h > 0 {
                self.upload(gpu, color, rect, &image.data)?;
            }
            Some(Placement {
                rect,
                left: image.placement.left,
                top: image.placement.top,
                color,
                page_generation: self
                    .pages_for_ref(color)
                    .get(rect.page)
                    .map_or(0, |page| page.generation),
            })
        }
    }

    pub fn binding(&self, placement: Placement) -> (AtlasBinding, u32) {
        let page = placement.rect.page;
        match &self.pages_for_ref(placement.color)[page].backing {
            PageBacking::Resident { layer, .. } => (AtlasBinding::Resident, *layer),
            PageBacking::Temporary { .. } => (
                AtlasBinding::Temporary {
                    color: placement.color,
                    page,
                },
                0,
            ),
        }
    }

    pub fn material(
        &mut self,
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        binding: AtlasBinding,
    ) -> (wgpu::BindGroup, bool) {
        let create = |mask: &wgpu::TextureView, color: &wgpu::TextureView| {
            crate::pipelines::bind_group(
                gpu,
                layout,
                &[
                    wgpu::BindingResource::TextureView(mask),
                    wgpu::BindingResource::TextureView(color),
                    wgpu::BindingResource::Sampler(sampler),
                ],
            )
        };
        let blank = &self.blank.view;
        let (slot, mask, color) = match binding {
            AtlasBinding::Resident => (
                &mut self.material,
                self.mask
                    .texture
                    .as_ref()
                    .map_or(blank, |texture| &texture.view),
                self.color
                    .texture
                    .as_ref()
                    .map_or(blank, |texture| &texture.view),
            ),
            AtlasBinding::Temporary { color, page } => {
                let atlas = if color {
                    &mut self.color
                } else {
                    &mut self.mask
                };
                let PageBacking::Temporary { texture, material } = &mut atlas.pages[page].backing
                else {
                    unreachable!("temporary atlas binding references a temporary page")
                };
                if color {
                    (material, blank, &texture.view)
                } else {
                    (material, &texture.view, blank)
                }
            }
        };
        let created = slot.is_none();
        (
            slot.get_or_insert_with(|| create(mask, color)).clone(),
            created,
        )
    }
}

impl AtlasArray {
    fn bytes(&self, channels: usize) -> usize {
        let size = |texture: &AtlasTexture| {
            texture.texture.width() as usize
                * texture.texture.height() as usize
                * texture.layers as usize
                * channels
        };
        self.texture.as_ref().map_or(0, size)
            + self
                .pages
                .iter()
                .map(|page| match &page.backing {
                    PageBacking::Temporary { texture, .. } => size(texture),
                    _ => 0,
                })
                .sum::<usize>()
    }
}

impl Page {
    fn temporary(&self) -> bool {
        matches!(self.backing, PageBacking::Temporary { .. })
    }
}

fn retain_pages(pages: &mut Vec<Page>) -> Vec<Option<usize>> {
    std::mem::take(pages)
        .into_iter()
        .map(|page| {
            if page.temporary() {
                return None;
            }
            let index = pages.len();
            pages.push(page);
            Some(index)
        })
        .collect()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite physical sizes are quantized to a fixed-size bucket"
)]
fn quantize_size(size: f32, max_size: u32, hinted: bool) -> Option<u32> {
    if !size.is_finite() || size <= 0.0 || size > max_size as f32 {
        return None;
    }
    let steps = if hinted { 8 } else { 4 };
    Some((size * steps as f32).round().max(1.0) as u32 * (64 / steps))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "subpixel offsets are quantized to four buckets"
)]
pub(crate) fn horizontal_bucket(fraction: f32, buckets: u32) -> u32 {
    let value = (fraction.rem_euclid(1.0) * buckets as f32).floor() as u32;
    value.min(buckets - 1) * (SUBPIXEL_BUCKETS / buckets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(gpu: &Gpu, texture: &wgpu::Texture, x: u32, y: u32, z: u32) -> u8 {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas test pixel"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut source = texture.as_image_copy();
        source.origin = wgpu::Origin3d { x, y, z };
        encoder.copy_texture_to_buffer(
            source,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: Default::default(),
            },
            crate::compositor::extent([1, 1]),
        );
        let submitted = gpu.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        let timeout = Some(std::time::Duration::from_secs(5));
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout,
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range().unwrap();
        mapped[0]
    }

    #[test]
    fn atlas_growth_patches_old_views_and_reclaims_a_cold_bucket() {
        let Some(raster) = crate::tests::headless(FontLibrary::new()) else {
            return;
        };
        let gpu = &raster.device.as_ref().unwrap().gpu;
        let mut atlas = Atlas::new(gpu);
        // Exercise growth and recycling with small pages and full-page glyphs.
        atlas.page_size = 1024;
        atlas.mask.texture = Some(AtlasTexture::new(gpu, false, [1024; 2], 1));
        let count = atlas.page_limit(gpu);
        let initial = atlas.mask.texture.as_ref().unwrap().texture.clone();
        atlas.begin_frame();
        let glyph = vec![200; 600 * 600 * atlas.mask_channels as usize];
        for layer in 0..count {
            let rect = atlas.allocate(gpu, false, 600, 600).unwrap();
            let placement = Placement {
                rect,
                left: 0,
                top: 0,
                color: false,
                page_generation: 0,
            };
            atlas.insert(
                GlyphKey {
                    face: Arc::new(FontFaceId(u64::from(layer))),
                    glyph: 0,
                    variant: GlyphVariant {
                        size: 1,
                        subpixel: 0,
                        darken_stem: false,
                        hint: false,
                    },
                    coords: None,
                    embolden: false,
                    skew: None,
                },
                placement,
                u64::from(layer),
            );
            atlas.pin(placement);
            atlas.upload(gpu, false, rect, &glyph).unwrap();
            let (binding, index) = atlas.binding(placement);
            assert!(binding == AtlasBinding::Resident);
            assert_eq!(index, layer);
        }
        let mut prelude = None;
        atlas.flush(gpu, &mut prelude);
        gpu.submit(prelude.map(wgpu::CommandEncoder::finish));
        assert_eq!(sample(gpu, &initial, 2, 2, 0), 200);
        let expanded = atlas.mask.texture.as_ref().unwrap().texture.clone();
        assert_eq!(sample(gpu, &expanded, 2, 2, 0), 200);
        assert_eq!(sample(gpu, &expanded, 2, 2, count - 1), 200);

        atlas.begin_frame();
        let new = atlas.allocate(gpu, false, 520, 520).unwrap();
        assert_eq!(new.page, 0);
        atlas
            .upload(
                gpu,
                false,
                new,
                &vec![100; 520 * 520 * atlas.mask_channels as usize],
            )
            .unwrap();
        let mut prelude = None;
        atlas.flush(gpu, &mut prelude);
        gpu.submit(prelude.map(wgpu::CommandEncoder::finish));
        assert_eq!(sample(gpu, &expanded, new.x + 3, new.y + 3, 0), 100);
        assert_eq!(sample(gpu, &expanded, new.x + new.w, new.y + 3, 0), 0);
        assert_eq!(atlas.stats.region_reclaims, 1);
        assert_eq!(atlas.entries.len(), (count - 1) as usize);
    }
}
