//! Size-independent distance fields for glyphs and reusable vector assets.

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
};

use engine::scene::{CanvasPath, PathVerb};
use etagere::{AllocId, AtlasAllocator, size2};
use fdsm::{
    bezier::{Segment, scanline::FillRule},
    generate::generate_mtsdf,
    render::correct_sign_mtsdf,
    shape::{Contour, Shape},
};
use rayon::prelude::*;
use swash::scale::ScaleContext;

use crate::{
    FontLibrary,
    atlas::AtlasStats,
    batch::{GLYPH_MSDF, GlyphVertex},
    compiler::{Affine, GlyphDraw},
    gpu::Gpu,
    outline::{OutlineKey, REFERENCE_PPEM},
    pipelines,
};

const PPEM: f32 = 48.0;
const MARGIN: f32 = 4.0;
const PAGE: u32 = 1024;
const LAYERS: u32 = 2;

#[derive(Default)]
pub(crate) struct MsdfAtlas {
    backing: Option<Backing>,
    prepared: HashMap<DistanceField, Option<Bitmap>>,
    unavailable: HashSet<DistanceField>,
    frame: u64,
    stats: AtlasStats,
}

struct Backing {
    texture: wgpu::Texture,
    groups: wgpu::BindGroup,
    pages: [AtlasAllocator; LAYERS as usize],
    entries: HashMap<DistanceField, Entry>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum DistanceField {
    Glyph(OutlineKey),
    Vector(VectorKey),
}

#[derive(Clone, Debug)]
pub(crate) struct VectorKey {
    hash: u64,
    path: CanvasPath,
}

impl PartialEq for VectorKey {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash && self.path == other.path
    }
}

impl Eq for VectorKey {}

impl Hash for VectorKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorField {
    key: VectorKey,
    pub transform: Affine,
    scale: f32,
}

impl VectorField {
    pub(crate) fn asset(&self) -> (VectorKey, f32) {
        (self.key.clone(), self.scale)
    }

    pub(crate) fn new(path: &CanvasPath, transform: Affine) -> Option<Self> {
        let (min, max) = bounds(path);
        let [w, h] = extent(min, max);
        let scale = 64.0 / w.max(h);
        if !scale.is_finite() || w.min(h) * scale < 4.0 {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        path.fill.hash(&mut hasher);
        for verb in path.verbs.iter() {
            std::mem::discriminant(verb).hash(&mut hasher);
            for point in points(*verb).into_iter().flatten() {
                for coord in point {
                    if !coord.is_finite() {
                        return None;
                    }
                    (if coord == 0.0 { 0 } else { coord.to_bits() }).hash(&mut hasher);
                }
            }
        }
        Some(Self {
            key: VectorKey {
                hash: hasher.finish(),
                path: path.clone(),
            },
            transform,
            scale,
        })
    }
}

struct Entry {
    allocation: AllocId,
    origin: [u16; 2],
    layer: u32,
    size: [u16; 2],
    bearing: [f32; 2],
    used: u64,
}

struct Bitmap {
    size: [u16; 2],
    bearing: [f32; 2],
    pixels: Vec<u8>,
}

impl MsdfAtlas {
    pub(crate) fn begin(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.stats = AtlasStats::default();
    }

    pub(crate) fn prepare(
        &mut self,
        pool: &rayon::ThreadPool,
        fonts: &FontLibrary,
        glyphs: HashSet<OutlineKey>,
        vectors: HashMap<VectorKey, f32>,
    ) {
        self.prepared.retain(|key, _| match key {
            DistanceField::Glyph(key) => glyphs.contains(key),
            DistanceField::Vector(key) => vectors.contains_key(key),
        });
        let missing: Vec<_> = glyphs
            .into_iter()
            .map(|key| (DistanceField::Glyph(key), PPEM / REFERENCE_PPEM))
            .chain(
                vectors
                    .into_iter()
                    .map(|(key, scale)| (DistanceField::Vector(key), scale)),
            )
            .filter(|(key, _)| {
                !self.prepared.contains_key(key)
                    && !self.unavailable.contains(key)
                    && self
                        .backing
                        .as_ref()
                        .is_none_or(|backing| !backing.entries.contains_key(key))
            })
            .collect();
        let prepared = pool.install(|| {
            missing
                .into_par_iter()
                .map_init(ScaleContext::new, |context, (key, scale)| {
                    let bitmap = match &key {
                        DistanceField::Glyph(outline) => outline
                            .path(fonts, context)
                            .and_then(|path| rasterize(&path, scale)),
                        DistanceField::Vector(asset) => rasterize(&asset.path, scale),
                    };
                    (key, bitmap)
                })
                .collect::<Vec<_>>()
        });
        self.prepared.extend(prepared);
    }

    pub(crate) fn glyph(
        &mut self,
        gpu: &Gpu,
        fonts: &FontLibrary,
        layouts: &pipelines::Layouts,
        sampler: &wgpu::Sampler,
        glyph: &GlyphDraw,
    ) -> Option<(GlyphVertex, bool)> {
        let outline = OutlineKey::new(glyph);
        self.lookup(
            gpu,
            layouts,
            sampler,
            DistanceField::Glyph(outline.clone()),
            || {
                outline
                    .path(fonts, &mut ScaleContext::new())
                    .and_then(|path| rasterize(&path, PPEM / REFERENCE_PPEM))
            },
            |entry| entry.vertex([glyph.x, glyph.y], glyph.size / PPEM, glyph.color),
        )
    }

    pub(crate) fn vector(
        &mut self,
        gpu: &Gpu,
        layouts: &pipelines::Layouts,
        sampler: &wgpu::Sampler,
        asset: &VectorField,
        color: [u16; 4],
    ) -> Option<(GlyphVertex, bool)> {
        self.lookup(
            gpu,
            layouts,
            sampler,
            DistanceField::Vector(asset.key.clone()),
            || rasterize(&asset.key.path, asset.scale),
            |entry| entry.vertex([0.0; 2], asset.scale.recip(), color),
        )
    }

    fn lookup(
        &mut self,
        gpu: &Gpu,
        layouts: &pipelines::Layouts,
        sampler: &wgpu::Sampler,
        key: DistanceField,
        bitmap: impl FnOnce() -> Option<Bitmap>,
        vertex: impl FnOnce(&Entry) -> GlyphVertex,
    ) -> Option<(GlyphVertex, bool)> {
        if let Some(entry) = self
            .backing
            .as_mut()
            .and_then(|backing| backing.entries.get_mut(&key))
        {
            self.stats.hits += 1;
            entry.used = self.frame;
            return Some((vertex(entry), false));
        }
        if self.unavailable.contains(&key) {
            return None;
        }
        let Some(bitmap) = self.prepared.remove(&key).unwrap_or_else(bitmap) else {
            if self.unavailable.len() >= 2048 {
                self.unavailable.clear();
            }
            self.unavailable.insert(key);
            return None;
        };
        let created = self.backing.is_none();
        let backing = self
            .backing
            .get_or_insert_with(|| Backing::new(gpu, layouts, sampler));
        let entry = backing.place(gpu, self.frame, &bitmap)?;
        let vertex = vertex(&entry);
        self.stats.misses += 1;
        self.stats.uploads += 1;
        self.stats.upload_bytes += bitmap.pixels.len() as u64;
        backing.entries.insert(key, entry);
        Some((vertex, created))
    }

    pub(crate) fn material(&self) -> Option<wgpu::BindGroup> {
        self.backing.as_ref().map(|backing| backing.groups.clone())
    }

    pub(crate) fn stats(&self) -> AtlasStats {
        self.stats
    }
}

impl Backing {
    fn new(gpu: &Gpu, layouts: &pipelines::Layouts, sampler: &wgpu::Sampler) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto distance field atlas"),
            size: wgpu::Extent3d {
                width: PAGE,
                height: PAGE,
                depth_or_array_layers: LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let groups = pipelines::bind_group(
            gpu,
            &layouts.atlas,
            &[
                wgpu::BindingResource::TextureView(&view),
                wgpu::BindingResource::TextureView(&view),
                wgpu::BindingResource::Sampler(sampler),
            ],
        );
        Self {
            texture,
            groups,
            pages: std::array::from_fn(|_| AtlasAllocator::new(size2(PAGE as i32, PAGE as i32))),
            entries: HashMap::new(),
        }
    }

    fn place(&mut self, gpu: &Gpu, frame: u64, bitmap: &Bitmap) -> Option<Entry> {
        let [w, h] = bitmap.size;
        let wanted = size2(i32::from(w) + 2, i32::from(h) + 2);
        let (layer, allocation) = loop {
            if let Some(allocation) = (0u32..).zip(&mut self.pages).find_map(|(layer, page)| {
                page.allocate(wanted).map(|allocation| (layer, allocation))
            }) {
                break allocation;
            }
            let key = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.used != frame)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())?;
            let evicted = self.entries.remove(&key)?;
            self.pages[evicted.layer as usize].deallocate(evicted.allocation);
        };
        let origin = [
            u16::try_from(allocation.rectangle.min.x + 1).ok()?,
            u16::try_from(allocation.rectangle.min.y + 1).ok()?,
        ];
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d {
                    x: u32::from(origin[0]),
                    y: u32::from(origin[1]),
                    z: layer,
                },
                ..self.texture.as_image_copy()
            },
            &bitmap.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(u32::from(w) * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w.into(),
                height: h.into(),
                depth_or_array_layers: 1,
            },
        );
        Some(Entry {
            origin,
            layer,
            allocation: allocation.id,
            size: bitmap.size,
            bearing: bitmap.bearing,
            used: frame,
        })
    }
}

impl Entry {
    fn vertex(&self, [px, py]: [f32; 2], scale: f32, color: [u16; 4]) -> GlyphVertex {
        let [x, y] = self.origin;
        let [w, h] = self.size;
        GlyphVertex {
            rect: [
                px + self.bearing[0] * scale,
                py + self.bearing[1] * scale,
                f32::from(w) * scale,
                f32::from(h) * scale,
            ],
            uv: [x, y, x + w, y + h],
            color,
            flags_and_layer: GLYPH_MSDF | (self.layer << 8),
            paint: u32::MAX,
        }
    }
}

fn points(verb: PathVerb) -> [Option<[f32; 2]>; 3] {
    match verb {
        PathVerb::Move(x, y) | PathVerb::Line(x, y) => [Some([x, y]), None, None],
        PathVerb::Quad(x, y, a, b) => [Some([x, y]), Some([a, b]), None],
        PathVerb::Cubic(x, y, a, b, c, d) => [Some([x, y]), Some([a, b]), Some([c, d])],
        PathVerb::Close => [None; 3],
    }
}

fn bounds(path: &CanvasPath) -> ([f32; 2], [f32; 2]) {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for [x, y] in path
        .verbs
        .iter()
        .flat_map(|verb| points(*verb).into_iter().flatten())
    {
        min = [min[0].min(x), min[1].min(y)];
        max = [max[0].max(x), max[1].max(y)];
    }
    (min, max)
}

fn extent(min: [f32; 2], max: [f32; 2]) -> [f32; 2] {
    [max[0] - min[0], max[1] - min[1]]
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "raster bounds are clamped before conversion"
)]
fn rasterize(path: &CanvasPath, scale: f32) -> Option<Bitmap> {
    let (min, max) = bounds(path);
    let min = min.map(|value| value * scale);
    let max = max.map(|value| value * scale);
    let bearing = min.map(|value| value.floor() - MARGIN);
    let extent = [
        (max[0] - bearing[0]).ceil() + MARGIN,
        (max[1] - bearing[1]).ceil() + MARGIN,
    ];
    if !extent.into_iter().all(|side| (1.0..=256.0).contains(&side)) {
        return None;
    }
    let point = |x, y| {
        [
            f64::from(x * scale - bearing[0]),
            f64::from(y * scale - bearing[1]),
        ]
        .into()
    };
    let mut shape = Shape::default();
    let mut contour = Contour::default();
    let mut first = point(0.0, 0.0);
    let mut last = first;
    let close = |shape: &mut Shape<Contour>, mut contour: Contour, last, first| {
        if !contour.segments.is_empty() {
            if last != first {
                contour.segments.push(Segment::line(last, first));
            }
            shape.contours.push(contour);
        }
    };
    for verb in path.verbs.iter() {
        let (segment, end) = match *verb {
            PathVerb::Move(x, y) => {
                close(&mut shape, std::mem::take(&mut contour), last, first);
                first = point(x, y);
                last = first;
                continue;
            }
            PathVerb::Close => {
                close(&mut shape, std::mem::take(&mut contour), last, first);
                last = first;
                continue;
            }
            PathVerb::Line(x, y) => (Segment::line(last, point(x, y)), point(x, y)),
            PathVerb::Quad(cx, cy, x, y) => {
                (Segment::quad(last, point(cx, cy), point(x, y)), point(x, y))
            }
            PathVerb::Cubic(cx, cy, dx, dy, x, y) => (
                Segment::cubic(last, point(cx, cy), point(dx, dy), point(x, y)),
                point(x, y),
            ),
        };
        if segment.start() != segment.end() || !matches!(verb, PathVerb::Line(..)) {
            contour.segments.push(segment);
        }
        last = end;
    }
    close(&mut shape, contour, last, first);
    if shape.contours.is_empty() {
        return None;
    }
    let size = extent.map(|side| side as u16);
    let mut image = image::RgbaImage::new(size[0].into(), size[1].into());
    let shape = Shape::edge_coloring_simple(shape, 0.03, 0).prepare();
    generate_mtsdf(&shape, f64::from(MARGIN * 2.0), &mut image);
    let rule = match path.fill {
        engine::scene::FillRule::NonZero => FillRule::Nonzero,
        engine::scene::FillRule::EvenOdd => FillRule::Odd,
    };
    correct_sign_mtsdf(&mut image, &shape, rule);
    Some(Bitmap {
        size,
        bearing,
        pixels: image.into_raw(),
    })
}
