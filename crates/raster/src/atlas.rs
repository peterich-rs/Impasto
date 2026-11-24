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
use etagere::size2;
use rayon::prelude::{ParallelIterator, ParallelSlice};
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Angle, Fill, Format, Style, Transform, Vector};
use swash::{CacheKey, FontRef, GlyphId};

use crate::compiler::GlyphDraw;
use crate::fonts::FontLibrary;
use crate::gpu::Gpu;

/// Quarter-pixel horizontal positions.
pub(crate) const SUBPIXEL_BUCKETS: u32 = 4;
const PAGE_SIZE: u32 = 2048;
/// Upper bound on simultaneously resident pages on small adapters.
const PAGE_LIMIT: u32 = 6;
/// One-texel gutter between glyphs so bilinear sampling never bleeds.
const GUTTER: u32 = 1;

/// Identity of one rasterized glyph bitmap.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    face: FontFaceId,
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
    lcd: bool,
}

impl GlyphKey {
    fn new(draw: &GlyphDraw, variant: GlyphVariant) -> Self {
        Self {
            face: *draw.face,
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
    pub lcd: bool,
    page_generation: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    variant: GlyphVariant,
    placement: Placement,
    used: u64,
}

const BAND_HEIGHT: u32 = 64;

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
            } else if gpu.caps.lcd {
                wgpu::TextureFormat::Rgba8Unorm
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
        shadow: Vec<u8>,
        dirty: u32,
    },
    Temporary {
        texture: AtlasTexture,
        material: Option<wgpu::BindGroup>,
    },
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
            mask_channels: if gpu.caps.lcd { 4 } else { 1 },
            epoch: 0,
            blank: AtlasTexture::new(gpu, true, [1, 1], 1),
            material: None,
            stats: AtlasStats::default(),
            scratch: Vec::new(),
            darken_small: std::env::var("IMPASTO_DARKEN_STEMS").as_deref() != Ok("0"),
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
        self.rasterizer.font_keys.clear();
        self.prepared.clear();
        self.scratch = Vec::new();
        self.pinned.clear();
        self.stats = AtlasStats::default();
    }

    /// Starts a transaction after the previous frame was submitted or discarded.
    pub fn begin_frame(&mut self) {
        self.prepared.clear();
        self.pinned.clear();
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

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn inverse_page_size(&self) -> f32 {
        1.0 / self.page_size as f32
    }

    pub fn resident_page_valid(&self, placement: Placement) -> bool {
        self.pages_for_ref(placement.color)
            .get(placement.rect.page)
            .is_some_and(|page| !page.temporary() && page.generation == placement.page_generation)
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
        position: Option<[f32; 2]>,
        lcd: bool,
    ) -> Option<Placement> {
        let variant = self.variant(gpu, size, position, lcd)?;
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
            .or_else(|| self.rasterizer.glyph(library, &key))?;
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
        let mut placement = entry.placement;
        if let Some(page) = self.pages_for_ref(placement.color).get(placement.rect.page) {
            placement.page_generation = page.generation;
        }
        self.pin(placement);
        Some(placement)
    }

    fn variant(
        &self,
        gpu: &Gpu,
        size: f32,
        position: Option<[f32; 2]>,
        lcd: bool,
    ) -> Option<GlyphVariant> {
        let size = quantize_size(size, gpu.max_texture_size())?;
        Some(GlyphVariant {
            size,
            subpixel: position.map_or(0, |[x, _]| horizontal_bucket(x)),
            darken_stem: self.darken_small && position.is_some() && size <= 14 * 64,
            hint: position.is_some(),
            lcd,
        })
    }

    pub(crate) fn missing_glyph(
        &self,
        gpu: &Gpu,
        draw: &GlyphDraw,
        size: f32,
        position: Option<[f32; 2]>,
        lcd: bool,
    ) -> Option<GlyphKey> {
        let variant = self.variant(gpu, size, position, lcd)?;
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
        pool: &rayon::ThreadPool,
        library: &FontLibrary,
        keys: HashSet<GlyphKey>,
    ) {
        if keys.len() < 4 {
            return;
        }
        let span = tracing::debug_span!(
            target: "impasto::compile",
            "parallel_prepare",
            pictures_rebuilt = 0u64,
            workers = pool.current_num_threads(),
            tess_us = 0u64,
            glyph_pre_us = tracing::field::Empty,
            glyphs = keys.len()
        );
        let _entered = span.enter();
        let started = std::time::Instant::now();
        let keys: Vec<_> = keys.into_iter().collect();
        let chunk_size = keys.len().div_ceil(pool.current_num_threads());
        let prepared = pool.install(|| {
            keys.par_chunks(chunk_size)
                .flat_map_iter(|chunk| {
                    let mut worker = GlyphRasterizer::default();
                    chunk.iter().filter_map(move |key| {
                        worker
                            .glyph(library, key)
                            .map(|bitmap| (key.clone(), bitmap))
                    })
                })
                .collect::<Vec<_>>()
        });
        self.stats.prerasterized += u32::try_from(prepared.len()).unwrap_or(u32::MAX);
        self.prepared.extend(prepared);
        span.record(
            "glyph_pre_us",
            u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        );
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
                lcd: false,
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
            let lcd = image.content == swash::scale::image::Content::SubpixelMask;
            if w > 0 && h > 0 {
                if !color && !lcd && self.mask_channels == 4 {
                    let rgba: Vec<_> = image.data.iter().flat_map(|&value| [value; 4]).collect();
                    self.upload(gpu, color, rect, &rgba)?;
                } else {
                    self.upload(gpu, color, rect, &image.data)?;
                }
            }
            Some(Placement {
                rect,
                left: image.placement.left,
                top: image.placement.top,
                color,
                lcd,
                page_generation: self
                    .pages_for_ref(color)
                    .get(rect.page)
                    .map_or(0, |page| page.generation),
            })
        }
    }

    fn allocate(&mut self, gpu: &Gpu, color: bool, w: u32, h: u32) -> Option<AtlasRect> {
        let empty = AtlasRect {
            page: usize::MAX,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            allocation: None,
        };
        if w == 0 || h == 0 {
            return Some(empty);
        }
        let required = w.max(h).checked_add(2 * GUTTER)?;
        if required > gpu.max_texture_size() {
            return None;
        }
        let page_size = self.page_size;
        if required > page_size {
            let page = self.add_page(gpu, color, [w + 2 * GUTTER, h + 2 * GUTTER], true);
            return Some(AtlasRect {
                page,
                x: GUTTER,
                y: GUTTER,
                w,
                h,
                allocation: None,
            });
        }
        let wanted = size2(w as i32 + GUTTER as i32 * 2, h as i32 + GUTTER as i32 * 2);
        let place = |allocation: etagere::Allocation, page: usize| AtlasRect {
            page,
            x: allocation.rectangle.min.x as u32 + GUTTER,
            y: allocation.rectangle.min.y as u32 + GUTTER,
            w,
            h,
            allocation: Some(allocation.id),
        };
        // Try existing pages.
        {
            let pages = self.pages_for(color);
            for (index, page) in pages.iter_mut().enumerate() {
                if page.size == [page_size; 2]
                    && let Some(allocation) = page.alloc.allocate(wanted)
                {
                    return Some(place(allocation, index));
                }
            }
        }
        let pages = self.pages_for_ref(color);
        let limit = self.page_limit(gpu) as usize;
        let full = pages.iter().filter(|page| !page.temporary()).count() >= limit;
        if full {
            let oldest = pages
                .iter()
                .enumerate()
                .filter(|(index, page)| {
                    !page.temporary() && !self.pinned.contains(&(color, *index))
                })
                .min_by_key(|(_, page)| page.age)
                .map(|(index, _)| index);
            if let Some(index) = oldest
                && let Some(allocation) = self.reclaim(color, index, wanted)
            {
                return Some(place(allocation, index));
            }
        }
        let index = self.add_page(gpu, color, [page_size; 2], full);
        let allocation = self
            .pages_for(color)
            .get_mut(index)?
            .alloc
            .allocate(wanted)?;
        Some(place(allocation, index))
    }

    fn page_limit(&self, gpu: &Gpu) -> u32 {
        // Budget each array for at most three full 2048-square layers.
        let area = self.page_size * self.page_size;
        (3 * PAGE_SIZE * PAGE_SIZE / area)
            .min(gpu.caps.max_array_layers)
            .min(PAGE_LIMIT)
    }

    fn add_page(&mut self, gpu: &Gpu, color: bool, size: [u32; 2], temporary: bool) -> usize {
        let max_layers = self.page_limit(gpu);
        let page_size = self.page_size;
        let age = self.clock;
        let atlas = if color {
            &mut self.color
        } else {
            &mut self.mask
        };
        let backing = if temporary {
            PageBacking::Temporary {
                texture: AtlasTexture::new(gpu, color, size, 1),
                material: None,
            }
        } else {
            let layer = atlas.pages.iter().filter(|page| !page.temporary()).count();
            let layer = u32::try_from(layer).unwrap_or(0);
            let current_layers = atlas.texture.as_ref().map_or(0, |texture| texture.layers);
            if layer >= current_layers {
                let layers = (layer + 1).next_power_of_two().min(max_layers);
                let next = AtlasTexture::new(gpu, color, [page_size; 2], layers);
                if let Some(old) = atlas.texture.replace(next) {
                    atlas.retired.push(old);
                }
                self.material = None;
                self.epoch = self.epoch.wrapping_add(1);
            }
            PageBacking::Resident {
                layer,
                shadow: if color {
                    Vec::new()
                } else {
                    vec![0; (page_size * page_size * self.mask_channels) as usize]
                },
                dirty: 0,
            }
        };
        let index = atlas.pages.len();
        atlas.pages.push(Page {
            alloc: BucketedAtlasAllocator::new(size2(size[0] as i32, size[1] as i32)),
            size,
            backing,
            age,
            generation: 0,
        });
        self.stats.bytes = self.mask.bytes(self.mask_channels as usize) + self.color.bytes(4);
        index
    }

    fn pages_for(&mut self, color: bool) -> &mut Vec<Page> {
        if color {
            &mut self.color.pages
        } else {
            &mut self.mask.pages
        }
    }

    fn pages_for_ref(&self, color: bool) -> &[Page] {
        if color {
            &self.color.pages
        } else {
            &self.mask.pages
        }
    }

    fn reclaim(
        &mut self,
        color: bool,
        index: usize,
        wanted: etagere::Size,
    ) -> Option<etagere::Allocation> {
        // A bucket is reused once its last glyph leaves. Bound the work per
        // attempt to leave enough time for a temporary page in this frame.
        const RECLAIM_BATCH: usize = 32;
        const RECLAIM_LIMIT: usize = 512;
        let mut victims: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(key, slot)| {
                let entry = self.slots.get(*slot)?;
                let placement = entry.placement;
                (placement.color == color && placement.rect.page == index)
                    .then(|| Some((entry.used, key.clone(), placement.rect.allocation?)))
                    .flatten()
            })
            .collect();
        victims.sort_unstable_by_key(|(used, _, _)| *used);
        if victims.is_empty() {
            return None;
        }
        let page = self.pages_for(color).get_mut(index)?;
        page.generation = page.generation.wrapping_add(1);
        for batch in victims[..victims.len().min(RECLAIM_LIMIT)].chunks(RECLAIM_BATCH) {
            for (_, key, allocation) in batch {
                if let Some(slot) = self.entries.remove(key) {
                    self.slots.remove(slot);
                }
                self.pages_for(color)[index].alloc.deallocate(*allocation);
            }
            self.stats.evictions += u32::try_from(batch.len()).unwrap_or(u32::MAX);
            self.stats.region_reclaims += 1;
            self.stats.entries = self.entries.len();
            if let Some(allocation) = self.pages_for(color)[index].alloc.allocate(wanted) {
                return Some(allocation);
            }
        }
        None
    }

    fn upload(&mut self, gpu: &Gpu, color: bool, rect: AtlasRect, data: &[u8]) -> Option<()> {
        let channels = if color { 4 } else { self.mask_channels };
        let row = (rect.w as usize).checked_mul(channels as usize)?;
        let atlas = if color {
            &mut self.color
        } else {
            &mut self.mask
        };
        let page = atlas.pages.get_mut(rect.page)?;
        if data.len() < row.checked_mul(rect.h as usize)? {
            return None;
        }
        let (texture, layer, mirror) = match &mut page.backing {
            PageBacking::Resident { shadow, dirty, .. } if !color => {
                let stride = (page.size[0] * channels) as usize;
                let gutter = GUTTER as usize * channels as usize;
                let padded_row = row + 2 * gutter;
                for y in rect.y - GUTTER..rect.y + rect.h + GUTTER {
                    let start = y as usize * stride + ((rect.x - GUTTER) * channels) as usize;
                    let padded = &mut shadow[start..start + padded_row];
                    padded.fill(0);
                    if let Some(row_index) = y
                        .checked_sub(rect.y)
                        .filter(|row_index| *row_index < rect.h)
                    {
                        let source = row_index as usize * row;
                        padded[gutter..gutter + row].copy_from_slice(&data[source..source + row]);
                    }
                }
                for band in
                    (rect.y - GUTTER) / BAND_HEIGHT..=(rect.y + rect.h + GUTTER - 1) / BAND_HEIGHT
                {
                    *dirty |= 1 << band;
                }
                return Some(());
            }
            PageBacking::Resident { layer, .. } => (atlas.texture.as_ref()?, *layer, true),
            PageBacking::Temporary { texture, .. } => (&*texture, 0, false),
        };
        let w = rect.w.checked_add(2 * GUTTER)?;
        let h = rect.h.checked_add(2 * GUTTER)?;
        let stride = zero_scratch(&mut self.scratch, [w, h], channels)?;
        for (y, source) in data.chunks_exact(row).take(rect.h as usize).enumerate() {
            let start = (y + GUTTER as usize) * stride + (GUTTER * channels) as usize;
            self.scratch[start..start + row].copy_from_slice(source);
        }
        // Queue writes precede any draw using this placement, including writes
        // to a color array that is subsequently copied into a larger array.
        for target in std::iter::once(texture).chain(
            atlas
                .retired
                .iter()
                .filter(|target| mirror && layer < target.layers),
        ) {
            self.stats.upload_bytes += write_texture(
                gpu,
                target,
                [rect.x - GUTTER, rect.y - GUTTER, layer],
                [w, h],
                channels,
                &self.scratch,
            );
            self.stats.upload_calls += 1;
        }
        Some(())
    }

    /// Upload once per contiguous band, mirroring patches to any array
    /// referenced by an earlier command buffer in the same transaction.
    pub fn flush(&mut self, gpu: &Gpu, prelude: &mut Option<wgpu::CommandEncoder>) {
        if let Some(texture) = &self.mask.texture {
            for page in &mut self.mask.pages {
                let PageBacking::Resident {
                    layer,
                    shadow,
                    dirty,
                } = &mut page.backing
                else {
                    continue;
                };
                let [w, h] = page.size;
                let bands = h.div_ceil(BAND_HEIGHT);
                let mut band = 0;
                while band < bands {
                    if *dirty & (1 << band) == 0 {
                        band += 1;
                        continue;
                    }
                    let y = band * BAND_HEIGHT;
                    while band < bands && *dirty & (1 << band) != 0 {
                        band += 1;
                    }
                    let end = (band * BAND_HEIGHT).min(h);
                    let stride = w * self.mask_channels;
                    let data = &shadow[(y * stride) as usize..(end * stride) as usize];
                    for target in std::iter::once(texture).chain(&self.mask.retired) {
                        if *layer < target.layers {
                            self.stats.upload_bytes += write_texture(
                                gpu,
                                target,
                                [0, y, *layer],
                                [w, end - y],
                                self.mask_channels,
                                data,
                            );
                            self.stats.upload_calls += 1;
                        }
                    }
                }
                *dirty = 0;
            }
        }
        for atlas in [&mut self.mask, &mut self.color] {
            for (old, next) in atlas
                .retired
                .iter()
                .zip(atlas.retired.iter().skip(1).chain(&atlas.texture))
            {
                // Writes are mirrored to both arrays before submit, so the copy
                // preserves new glyphs and brings previously uploaded layers forward.
                crate::arena::upload_encoder(gpu, prelude).copy_texture_to_texture(
                    old.texture.as_image_copy(),
                    next.texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: self.page_size,
                        height: self.page_size,
                        depth_or_array_layers: old.layers,
                    },
                );
                self.stats.copy_pixels += u64::from(self.page_size).pow(2) * u64::from(old.layers);
            }
            atlas.retired.clear();
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

fn zero_scratch(scratch: &mut Vec<u8>, size: [u32; 2], channels: u32) -> Option<usize> {
    let stride = (size[0] as usize).checked_mul(channels as usize)?;
    let length = stride.checked_mul(size[1] as usize)?;
    scratch.clear();
    scratch.try_reserve_exact(length).ok()?;
    scratch.resize(length, 0);
    Some(stride)
}

fn write_texture(
    gpu: &Gpu,
    texture: &AtlasTexture,
    origin: [u32; 3],
    size: [u32; 2],
    channels: u32,
    data: &[u8],
) -> u64 {
    let [x, y, z] = origin;
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * channels),
            rows_per_image: Some(size[1]),
        },
        crate::compositor::extent(size),
    );
    u64::from(size[0]) * u64::from(size[1]) * u64::from(channels)
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

#[derive(Default)]
struct GlyphRasterizer {
    context: ScaleContext,
    font_keys: HashMap<FontFaceId, CacheKey>,
}

impl GlyphRasterizer {
    fn glyph(&mut self, library: &FontLibrary, key: &GlyphKey) -> Option<Bitmap> {
        library.with_face(&key.face, |bytes, index| self.rasterize(bytes, index, key))?
    }

    fn rasterize(&mut self, bytes: &[u8], index: u32, key: &GlyphKey) -> Option<Bitmap> {
        let mut font = FontRef::from_index(bytes, index as usize)?;
        if self.font_keys.len() >= 1024 {
            self.font_keys.clear();
        }
        font.key = *self.font_keys.entry(key.face).or_insert(font.key);
        let ppem = key.variant.size as f32 / 64.0;
        let mut scaler = self
            .context
            .builder(font)
            .size(ppem)
            .hint(key.variant.hint)
            .normalized_coords(key.coords.as_deref().unwrap_or_default().iter().copied())
            .build();
        let offset = Vector::new(key.variant.subpixel as f32 / SUBPIXEL_BUCKETS as f32, 0.0);
        let id = GlyphId::from(key.glyph);
        // Color sources first: emoji and COLR glyphs keep their own pixels.
        let mut image = swash::scale::image::Image::new();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            Source::Bitmap(swash::scale::StrikeWith::ExactSize),
        ]);
        render.format(Format::Alpha).offset(offset);
        if render.render_into(&mut scaler, id, &mut image)
            && image.content == swash::scale::image::Content::Color
        {
            return Some(Bitmap::Color(image));
        }
        let mut render = Render::new(&[
            Source::Outline,
            Source::Bitmap(swash::scale::StrikeWith::ExactSize),
        ]);
        render
            .format(if key.variant.lcd {
                Format::Subpixel
            } else {
                Format::Alpha
            })
            .style(Style::Fill(Fill::NonZero))
            .offset(offset);
        if key.embolden {
            render.embolden(ppem / 24.0);
        } else if key.variant.darken_stem {
            render.embolden(0.12);
        }
        if let Some(skew) = key.skew.map(f32::from_bits) {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(-skew),
                Angle::ZERO,
            )));
        }
        if render.render_into(&mut scaler, id, &mut image) {
            return Some(Bitmap::Mask(image));
        }
        // No outline: whitespace or a missing glyph. Cache an empty entry.
        Some(Bitmap::Empty)
    }
}

enum Bitmap {
    Empty,
    Mask(swash::scale::image::Image),
    Color(swash::scale::image::Image),
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
fn quantize_size(size: f32, max_size: u32) -> Option<u32> {
    if !size.is_finite() || size <= 0.0 || size > max_size as f32 {
        return None;
    }
    Some((size * 64.0).round().max(1.0) as u32)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "subpixel offsets are quantized to four buckets"
)]
pub(crate) fn horizontal_bucket(fraction: f32) -> u32 {
    let value = (fraction.rem_euclid(1.0) * SUBPIXEL_BUCKETS as f32).floor();
    (value as i32).clamp(0, SUBPIXEL_BUCKETS as i32 - 1) as u32
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
                lcd: false,
                page_generation: 0,
            };
            atlas.insert(
                GlyphKey {
                    face: FontFaceId(u64::from(layer)),
                    glyph: 0,
                    variant: GlyphVariant {
                        size: 1,
                        subpixel: 0,
                        darken_stem: false,
                        hint: false,
                        lcd: false,
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
