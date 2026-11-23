//! Glyph atlases. Pages of `R8Unorm` coverage for ordinary antialiased
//! text and `Rgba8UnormSrgb` for color glyphs (emoji, COLR, bitmap strikes).
//!
//! Swash rasterizes on the raster thread. A glyph's key covers every input
//! that changes its bitmap: face, glyph ID, physical size, subpixel
//! position bucket, variation coordinates, and synthetic bold or skew.
//! Entries are shelf-packed. The shelf allocator cannot free one rect, so
//! eviction is per page: when every page is full and the page budget is
//! reached, the oldest page is cleared and its entries dropped.
//!
//! Blank glyphs (whitespace, missing outlines) cache an empty entry.

use std::collections::{HashMap, HashSet};

use engine::scene::FontFaceId;
use etagere::BucketedAtlasAllocator;
use etagere::size2;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Angle, Fill, Format, Style, Transform, Vector};
use swash::{CacheKey, FontRef, GlyphId};

use crate::compiler::GlyphDraw;
use crate::fonts::FontLibrary;
use crate::gpu::Gpu;

/// Subpixel buckets on each axis: quarter-pixel positioning.
const SUBPIXEL_BUCKETS: u32 = 4;
const PAGE_SIZE: u32 = 1024;
/// Pages per atlas above which the oldest page is recycled.
const PAGE_LIMIT: usize = 6;
/// One-texel gutter between glyphs so bilinear sampling never bleeds.
const GUTTER: u32 = 1;

/// Identity of one rasterized glyph bitmap.
#[derive(Clone, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: FontFaceId,
    glyph: u16,
    /// Physical font size in 1/64 pixel steps.
    size: u32,
    /// Subpixel bucket x and y, `0..SUBPIXEL_BUCKETS`.
    subpixel: [u32; 2],
    coords: Box<[i16]>,
    embolden: bool,
    hint: bool,
    skew: Option<u32>,
}

/// Texel rectangle of a glyph in one atlas page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
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
}

#[derive(Clone, Copy, Debug)]
struct Entry {
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
}

enum PageBacking {
    Resident {
        layer: u32,
        shadow: Vec<u8>,
        dirty: u16,
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
    context: ScaleContext,
    font_keys: HashMap<FontFaceId, CacheKey>,
    sources: Vec<Source>,
    mask: AtlasArray,
    color: AtlasArray,
    entries: HashMap<GlyphKey, Entry>,
    clock: u64,
    page_size: u32,
    blank: AtlasTexture,
    material: Option<wgpu::BindGroup>,
    stats: AtlasStats,
    scratch: Vec<u8>,
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
    /// Live entries across both atlases.
    pub entries: usize,
    /// Bytes of texture memory in live pages.
    pub bytes: usize,
}

impl Atlas {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            context: ScaleContext::new(),
            font_keys: HashMap::new(),
            sources: vec![
                Source::ColorOutline(0),
                Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
                Source::Bitmap(swash::scale::StrikeWith::ExactSize),
            ],
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
            clock: 0,
            page_size: gpu.max_texture_size().min(PAGE_SIZE),
            blank: AtlasTexture::new(gpu, true, [1, 1], 1),
            material: None,
            stats: AtlasStats::default(),
            scratch: Vec::new(),
            pinned: HashSet::new(),
        }
    }

    pub fn trim(&mut self) {
        self.mask = AtlasArray::default();
        self.color = AtlasArray::default();
        self.material = None;
        self.entries.clear();
        self.font_keys.clear();
        self.scratch = Vec::new();
        self.pinned.clear();
        self.stats = AtlasStats::default();
    }

    /// Starts a transaction after the previous frame was submitted or discarded.
    pub fn begin_frame(&mut self) {
        self.pinned.clear();
        // Retired arrays belonged to a submitted or discarded transaction.
        self.mask.retired.clear();
        self.color.retired.clear();
        let masks = retain_pages(&mut self.mask.pages);
        let colors = retain_pages(&mut self.color.pages);
        let before = self.entries.len();
        self.entries.retain(|_, entry| {
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
        self.stats.evictions = u32::try_from(before - self.entries.len()).unwrap_or(u32::MAX);
        if self.entries.len() > 32768 {
            self.entries
                .retain(|_, entry| entry.placement.rect.page != usize::MAX);
        }
        self.stats.bytes = self.mask.bytes(1) + self.color.bytes(4);
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

    fn pin(&mut self, placement: Placement) {
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
        self.stats.hits = 0;
        self.stats.misses = 0;
        self.stats.uploads = 0;
        self.stats.upload_calls = 0;
        self.stats.evictions = 0;
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
    ) -> Option<Placement> {
        let size = quantize_size(size, gpu.max_texture_size())?;
        let key = GlyphKey {
            face: *draw.face,
            glyph: draw.glyph,
            size,
            subpixel: position.map_or([0, 0], |position| position.map(bucket)),
            coords: draw.coords.to_vec().into(),
            embolden: draw.embolden,
            hint: position.is_some(),
            skew: draw.skew.map(f32::to_bits),
        };
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            self.stats.hits = self.stats.hits.saturating_add(1);
            let placement = entry.placement;
            self.pin(placement);
            return Some(placement);
        }
        self.stats.misses = self.stats.misses.saturating_add(1);
        let bitmap =
            library.with_face(&key.face, |bytes, index| self.rasterize(bytes, index, &key))??;
        let placement = self.store(gpu, key, bitmap)?;
        if placement.rect.page != usize::MAX {
            self.stats.uploads = self.stats.uploads.saturating_add(1);
        }
        self.pin(placement);
        Some(placement)
    }

    fn rasterize(&mut self, bytes: &[u8], index: u32, key: &GlyphKey) -> Option<Bitmap> {
        let mut font = FontRef::from_index(bytes, index as usize)?;
        if self.font_keys.len() >= 1024 {
            self.font_keys.clear();
        }
        font.key = *self.font_keys.entry(key.face).or_insert(font.key);
        let ppem = key.size as f32 / 64.0;
        let mut scaler = self
            .context
            .builder(font)
            .size(ppem)
            .hint(key.hint)
            .normalized_coords(key.coords.iter().copied())
            .build();
        let offset = Vector::new(
            key.subpixel[0] as f32 / SUBPIXEL_BUCKETS as f32,
            -(key.subpixel[1] as f32) / SUBPIXEL_BUCKETS as f32,
        );
        let id = GlyphId::from(key.glyph);
        // Color sources first: emoji and COLR glyphs keep their own pixels.
        let mut image = swash::scale::image::Image::new();
        let mut render = Render::new(&self.sources);
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
            .format(Format::Alpha)
            .style(Style::Fill(Fill::NonZero))
            .offset(offset);
        if key.embolden {
            render.embolden(ppem / 24.0);
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

    fn store(&mut self, gpu: &Gpu, key: GlyphKey, bitmap: Bitmap) -> Option<Placement> {
        let placement = match bitmap {
            Bitmap::Empty => Placement {
                rect: AtlasRect {
                    page: usize::MAX,
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                },
                left: 0,
                top: 0,
                color: false,
            },
            Bitmap::Mask(image) => self.store_image(gpu, false, image)?,
            Bitmap::Color(image) => self.store_image(gpu, true, image)?,
        };
        self.entries.insert(
            key,
            Entry {
                placement,
                used: self.clock,
            },
        );
        self.stats.entries = self.entries.len();
        Some(placement)
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
            });
        }
        let wanted = size2(w as i32 + GUTTER as i32 * 2, h as i32 + GUTTER as i32 * 2);
        let place = |allocation: etagere::Allocation, page: usize| AtlasRect {
            page,
            x: allocation.rectangle.min.x as u32 + GUTTER,
            y: allocation.rectangle.min.y as u32 + GUTTER,
            w,
            h,
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
        // Every page is full. Recycle the one whose entries were used
        // longest ago, or add a page while under the budget.
        let pages = self.pages_for_ref(color);
        let limit = PAGE_LIMIT.min(gpu.caps.max_array_layers as usize);
        let full = pages.iter().filter(|page| !page.temporary()).count() >= limit;
        let recycle = full
            .then(|| {
                pages
                    .iter()
                    .enumerate()
                    .filter(|(index, page)| {
                        !page.temporary() && !self.pinned.contains(&(color, *index))
                    })
                    .min_by_key(|(_, page)| page.age)
                    .map(|(index, _)| index)
            })
            .flatten();
        let index = if let Some(index) = recycle {
            self.recycle(gpu, color, index)?;
            index
        } else {
            self.add_page(gpu, color, [page_size; 2], full)
        };
        let allocation = self
            .pages_for(color)
            .get_mut(index)?
            .alloc
            .allocate(wanted)?;
        Some(place(allocation, index))
    }

    fn add_page(&mut self, gpu: &Gpu, color: bool, size: [u32; 2], temporary: bool) -> usize {
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
                let max_layers = gpu.caps.max_array_layers.min(6);
                let layers = (layer + 1).next_power_of_two().min(max_layers);
                let next = AtlasTexture::new(gpu, color, [page_size; 2], layers);
                if let Some(old) = atlas.texture.replace(next) {
                    let mut encoder =
                        gpu.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("grow glyph atlas"),
                            });
                    if let Some(next) = &atlas.texture {
                        encoder.copy_texture_to_texture(
                            old.texture.as_image_copy(),
                            next.texture.as_image_copy(),
                            wgpu::Extent3d {
                                width: page_size,
                                height: page_size,
                                depth_or_array_layers: old.layers,
                            },
                        );
                    }
                    self.stats.copy_pixels += u64::from(page_size).pow(2) * u64::from(old.layers);
                    gpu.queue.submit([encoder.finish()]);
                    if !color {
                        atlas.retired.push(old);
                    }
                }
                self.material = None;
            }
            PageBacking::Resident {
                layer,
                shadow: if color {
                    Vec::new()
                } else {
                    vec![0; (page_size * page_size) as usize]
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
        });
        self.stats.bytes = self.mask.bytes(1) + self.color.bytes(4);
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

    fn recycle(&mut self, gpu: &Gpu, color: bool, index: usize) -> Option<()> {
        let size = self.pages_for_ref(color).get(index)?.size;
        if color {
            zero_scratch(&mut self.scratch, size, 4)?;
        }
        let before = self.entries.len();
        self.entries.retain(|_, entry| {
            entry.placement.color != color || entry.placement.rect.page != index
        });
        self.stats.evictions = self
            .stats
            .evictions
            .saturating_add(u32::try_from(before - self.entries.len()).unwrap_or(u32::MAX));
        self.stats.entries = self.entries.len();
        let atlas = if color {
            &mut self.color
        } else {
            &mut self.mask
        };
        let page = &mut atlas.pages[index];
        page.alloc.clear();
        page.age = self.clock;
        if let PageBacking::Resident {
            layer,
            shadow,
            dirty,
        } = &mut page.backing
        {
            shadow.fill(0);
            if color {
                if let Some(texture) = &atlas.texture {
                    let [w, h] = page.size;
                    self.stats.upload_bytes +=
                        write_texture(gpu, texture, [0, 0, *layer], [w, h], 4, &self.scratch);
                    self.stats.upload_calls += 1;
                }
            } else {
                *dirty = u16::MAX;
            }
        }
        Some(())
    }

    fn upload(&mut self, gpu: &Gpu, color: bool, rect: AtlasRect, data: &[u8]) -> Option<()> {
        let channels = if color { 4 } else { 1 };
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
        let (texture, layer) = match &mut page.backing {
            PageBacking::Resident { shadow, dirty, .. } if !color => {
                let stride = page.size[0] as usize;
                for (y, source) in data.chunks_exact(row).take(rect.h as usize).enumerate() {
                    let start = (rect.y as usize + y) * stride + rect.x as usize;
                    shadow[start..start + row].copy_from_slice(source);
                }
                for band in rect.y / BAND_HEIGHT..=(rect.y + rect.h - 1) / BAND_HEIGHT {
                    *dirty |= 1 << band;
                }
                return Some(());
            }
            PageBacking::Resident { layer, .. } => (atlas.texture.as_ref()?, *layer),
            PageBacking::Temporary { texture, .. } => (&*texture, 0),
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
        self.stats.upload_bytes += write_texture(
            gpu,
            texture,
            [rect.x - GUTTER, rect.y - GUTTER, layer],
            [w, h],
            channels,
            &self.scratch,
        );
        self.stats.upload_calls += 1;
        Some(())
    }

    /// Upload once per contiguous band, mirroring patches to any array
    /// referenced by an earlier command buffer in the same transaction.
    pub fn flush(&mut self, gpu: &Gpu) {
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
                    let data = &shadow[(y * w) as usize..(end * w) as usize];
                    for target in std::iter::once(texture).chain(&self.mask.retired) {
                        if *layer < target.layers {
                            self.stats.upload_bytes +=
                                write_texture(gpu, target, [0, y, *layer], [w, end - y], 1, data);
                            self.stats.upload_calls += 1;
                        }
                    }
                }
                *dirty = 0;
            }
        }
        self.mask.retired.clear();
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
fn bucket(fraction: f32) -> u32 {
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
        let submitted = gpu.queue.submit([encoder.finish()]);
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
    fn atlas_growth_patches_old_views_and_recycling_zeros_the_full_layer() {
        let Some(raster) = crate::tests::headless(FontLibrary::new()) else {
            return;
        };
        let gpu = &raster.device.as_ref().unwrap().gpu;
        let mut atlas = Atlas::new(gpu);
        let count = PAGE_LIMIT.min(gpu.caps.max_array_layers as usize);
        let initial = atlas.mask.texture.as_ref().unwrap().texture.clone();
        atlas.begin_frame();
        let glyph = vec![200; 600 * 600];
        for layer in 0..count {
            let rect = atlas.allocate(gpu, false, 600, 600).unwrap();
            let placement = Placement {
                rect,
                left: 0,
                top: 0,
                color: false,
            };
            atlas.pin(placement);
            atlas.upload(gpu, false, rect, &glyph).unwrap();
            let (binding, index) = atlas.binding(placement);
            assert!(binding == AtlasBinding::Resident);
            assert_eq!(index, u32::try_from(layer).unwrap());
        }
        atlas.flush(gpu);
        assert_eq!(sample(gpu, &initial, 2, 2, 0), 200);
        let expanded = atlas.mask.texture.as_ref().unwrap().texture.clone();
        assert_eq!(sample(gpu, &expanded, 2, 2, 0), 200);
        assert_eq!(
            sample(gpu, &expanded, 2, 2, u32::try_from(count - 1).unwrap()),
            200
        );

        atlas.begin_frame();
        let new = atlas.allocate(gpu, false, 520, 520).unwrap();
        assert_eq!(new.page, 0);
        atlas
            .upload(gpu, false, new, &vec![100; 520 * 520])
            .unwrap();
        atlas.flush(gpu);
        assert_eq!(sample(gpu, &expanded, new.x + 3, new.y + 3, 0), 100);
        assert_eq!(sample(gpu, &expanded, new.x + new.w, new.y + 3, 0), 0);
        assert_eq!(sample(gpu, &expanded, 580, 580, 0), 0);
    }
}
