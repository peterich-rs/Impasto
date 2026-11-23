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

struct Page {
    alloc: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    temporary: bool,
    bytes: usize,
    /// Clock when this page was last recycled.
    age: u64,
}

/// Rasterizer for glyph bitmaps and the pages that hold them.
pub struct Atlas {
    context: ScaleContext,
    font_keys: HashMap<FontFaceId, CacheKey>,
    sources: Vec<Source>,
    pages: Vec<Page>,
    color_pages: Vec<Page>,
    entries: HashMap<GlyphKey, Entry>,
    clock: u64,
    page_size: u32,
    blank: wgpu::TextureView,
    stats: AtlasStats,
    /// Glyph bitmaps rasterized this frame, written in one flush.
    uploads: Vec<Upload>,
    pinned: HashSet<(bool, usize)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    pub hits: u32,
    pub misses: u32,
    pub uploads: u32,
    pub evictions: u32,
    /// Live entries across both atlases.
    pub entries: usize,
    /// Bytes of texture memory in live pages.
    pub bytes: usize,
}

impl Atlas {
    pub fn new(gpu: &Gpu, blank: wgpu::TextureView) -> Self {
        Self {
            context: ScaleContext::new(),
            font_keys: HashMap::new(),
            sources: vec![
                Source::ColorOutline(0),
                Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
                Source::Bitmap(swash::scale::StrikeWith::ExactSize),
            ],
            pages: Vec::new(),
            color_pages: Vec::new(),
            entries: HashMap::new(),
            clock: 0,
            page_size: gpu.max_texture_size().min(PAGE_SIZE),
            blank,
            stats: AtlasStats::default(),
            uploads: Vec::new(),
            pinned: HashSet::new(),
        }
    }

    pub fn trim(&mut self) {
        self.pages.clear();
        self.color_pages.clear();
        self.entries.clear();
        self.font_keys.clear();
        self.uploads.clear();
        self.pinned.clear();
        self.stats = AtlasStats::default();
    }

    /// Starts a transaction after the previous frame was submitted or discarded.
    pub fn begin_frame(&mut self) {
        self.pinned.clear();
        if !self.uploads.is_empty() {
            // Missing uploads mean the previous encode failed before submit.
            self.trim();
        }
        let masks = retain_pages(&mut self.pages);
        let colors = retain_pages(&mut self.color_pages);
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
        self.stats.bytes = self
            .pages
            .iter()
            .chain(&self.color_pages)
            .map(|page| page.bytes)
            .sum();
        self.stats.entries = self.entries.len();
    }

    pub fn uv(&self, placement: Placement) -> [f32; 4] {
        let rect = placement.rect;
        let [width, height] = self
            .pages_for_ref(placement.color)
            .get(rect.page)
            .map_or([1; 2], |page| page.size);
        [
            rect.x as f32 / width as f32,
            rect.y as f32 / height as f32,
            (rect.x + rect.w) as f32 / width as f32,
            (rect.y + rect.h) as f32 / height as f32,
        ]
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
                self.upload(color, rect, &image.data, w, h);
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
        let full = pages.iter().filter(|page| !page.temporary).count() >= PAGE_LIMIT;
        let recycle = full
            .then(|| {
                pages
                    .iter()
                    .enumerate()
                    .filter(|(index, page)| {
                        !page.temporary && !self.pinned.contains(&(color, *index))
                    })
                    .min_by_key(|(_, page)| page.age)
                    .map(|(index, _)| index)
            })
            .flatten();
        let index = if let Some(index) = recycle {
            self.recycle(color, index);
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
        let format = if color {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::R8Unorm
        };
        let page = Page::new(gpu, format, size, self.clock, temporary);
        self.stats.bytes = self.stats.bytes.saturating_add(page.bytes);
        let pages = self.pages_for(color);
        let index = pages.len();
        pages.push(page);
        index
    }

    fn pages_for(&mut self, color: bool) -> &mut Vec<Page> {
        if color {
            &mut self.color_pages
        } else {
            &mut self.pages
        }
    }

    /// Clears every entry on `index` and resets its allocator.
    fn recycle(&mut self, color: bool, index: usize) {
        let before = self.entries.len();
        self.entries.retain(|_, entry| {
            entry.placement.color != color || entry.placement.rect.page != index
        });
        let clock = self.clock;
        self.stats.evictions = self
            .stats
            .evictions
            .saturating_add(u32::try_from(before - self.entries.len()).unwrap_or(u32::MAX));
        self.stats.entries = self.entries.len();
        let pages = self.pages_for(color);
        if let Some(page) = pages.get_mut(index) {
            page.alloc.clear();
            page.age = clock;
        }
    }

    fn upload(&mut self, color: bool, rect: AtlasRect, data: &[u8], w: u32, h: u32) {
        if rect.page >= self.pages_for_ref(color).len() {
            return;
        }
        let bytes_per_pixel: u32 = if color { 4 } else { 1 };
        if data.len() < (w * h) as usize * bytes_per_pixel as usize {
            return;
        }
        self.uploads.push(Upload {
            color,
            rect,
            data: data.to_vec(),
            w,
            h,
            bytes_per_pixel,
        });
    }

    /// Flushes queued glyph uploads. Called once per frame after every
    /// glyph was placed and before the frame is submitted.
    pub fn flush(&mut self, gpu: &Gpu) {
        let uploads = std::mem::take(&mut self.uploads);
        for upload in uploads {
            let Some(page) = self.pages_for_ref(upload.color).get(upload.rect.page) else {
                continue;
            };
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &page.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: upload.rect.x,
                        y: upload.rect.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.w * upload.bytes_per_pixel),
                    rows_per_image: Some(upload.h),
                },
                wgpu::Extent3d {
                    width: upload.w,
                    height: upload.h,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn pages_for_ref(&self, color: bool) -> &Vec<Page> {
        if color {
            &self.color_pages
        } else {
            &self.pages
        }
    }

    /// The mask page view, or the blank view when nothing landed there.
    pub fn mask_view(&self, page: usize) -> &wgpu::TextureView {
        self.pages.get(page).map_or(&self.blank, |page| &page.view)
    }

    pub fn color_view(&self, page: usize) -> &wgpu::TextureView {
        self.color_pages
            .get(page)
            .map_or(&self.blank, |page| &page.view)
    }
}

struct Upload {
    color: bool,
    rect: AtlasRect,
    data: Vec<u8>,
    w: u32,
    h: u32,
    bytes_per_pixel: u32,
}

enum Bitmap {
    Empty,
    Mask(swash::scale::image::Image),
    Color(swash::scale::image::Image),
}

impl Page {
    fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        age: u64,
        temporary: bool,
    ) -> Self {
        let bytes_per_pixel = if format == wgpu::TextureFormat::Rgba8UnormSrgb {
            4
        } else {
            1
        };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto atlas page"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            alloc: BucketedAtlasAllocator::new(size2(size[0] as i32, size[1] as i32)),
            texture,
            view,
            size,
            temporary,
            bytes: size[0] as usize * size[1] as usize * bytes_per_pixel,
            age,
        }
    }
}

fn retain_pages(pages: &mut Vec<Page>) -> Vec<Option<usize>> {
    std::mem::take(pages)
        .into_iter()
        .map(|page| {
            if page.temporary {
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
