//! Ordered scene replay, clipping, and offscreen composition.

mod canvas;
mod clip;
mod draw;
mod draw_list;
mod effects;
mod glyph_prepare;
mod instances;
mod layers;
mod output;
mod passes;
mod picture;
mod runtime_effect;

use crate::profiler::Capture;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use engine::scene::{LayerId, LayerTree, PictureId};

use bytemuck::Zeroable;

use crate::arena::{Address, DynamicGroup, Extra, FrameArena, PaintPage, VertexSlice};
use crate::atlas::{Atlas, AtlasBinding, AtlasStats, SubpixelPosition};
use crate::batch::{self, GlyphVertex, PaintExtra, RectInstance};
use crate::compiler::{self, Affine, CompiledPicture, Group, PictureBounds};
use crate::compute_blur::ComputeBlur;
use crate::gpu::Gpu;
use crate::pipelines::{
    self, BlendKey, ParamsUniform, PipelineCache, PipelineId, PipelineKind, Target, ViewUniform,
};
use crate::{FontLibrary, Status, textures::TextureStore};
use clip::{add_corner_clip, add_rect_clip, merge_clip};
use draw_list::DrawList;
use effects::FullPass;
pub(crate) use glyph_prepare::{GlyphChanges, GlyphScene};
pub(crate) use output::{OutputFormat, PreparedOutput};
use passes::{DeferredRootPass, DrawPass, PassInputs, frame_encoder, pass_timestamps};
type Attachment = wgpu::TextureView;

const POOL_LIMIT: usize = 48;
const MASK_LIMIT: usize = 16;
const POOL_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct Compositor {
    atlas: Atlas,
    glyph_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    glyph_prefetch_deadline: Option<std::time::Instant>,
    has_submitted: bool,
    pipelines: PipelineCache,
    warm_formats: Vec<wgpu::TextureFormat>,
    pool: AttachmentPool,
    white: wgpu::TextureView,
    white_mask_group: wgpu::BindGroup,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    render_stats: RenderStats,
    arena: FrameArena,
    draw_lists: Vec<DrawList>,
    last_viewport: Option<[u32; 2]>,
    runtime_effects: crate::runtime_effect::RuntimeEffects,
    runtime_texture_groups: HashMap<
        [wgpu::TextureView; engine::scene::MAX_SHADER_TEXTURES],
        runtime_effect::TextureGroup,
    >,
    pub(super) effect_seconds: f32,
    image_groups: HashMap<(Vec<wgpu::TextureView>, bool), wgpu::BindGroup>,
    composites: HashMap<(wgpu::TextureView, wgpu::TextureView), wgpu::BindGroup>,
    masks: HashMap<MaskKey, Image>,
    offscreen_live_bytes: u64,
    frame_active: bool,
    trim_pending: bool,
    specialize_paint: bool,
    compute_blur: [Option<Option<ComputeBlur>>; 2],
    blur_groups: HashMap<(wgpu::TextureView, wgpu::TextureView, wgpu::Buffer), wgpu::BindGroup>,
    transient: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MaskKey {
    area: [u32; 4],
    scale: u32,
    shapes: Vec<MaskShapeKey>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MaskShapeKey {
    rect: [u32; 4],
    radius: u32,
    corners: Option<[u32; 5]>,
    transform: [u32; 6],
    mesh: Option<(MaskMeshKey, std::ops::Range<u32>)>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum MaskMeshKey {
    Resident(VertexSlice),
    Streamed(u64),
}

/// CPU-side encoding counters from the last rendered frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub draws: u32,
    pub passes: u32,
    pub pooled_texture_bytes: u64,
    pub buffers_created: u32,
    pub bind_groups_created: u32,
    pub vertex_bytes: u64,
    pub uniform_bytes: u64,
    pub stop_bytes: u64,
    pub extra_bytes: u64,
    pub mesh_upload_bytes: u64,
    pub atlas_upload_bytes: u64,
    pub offscreen_alloc_bytes: u64,
    pub offscreen_peak_bytes: u64,
    pub copy_pixels: u64,
    pub blur_pixels: u64,
    pub compute_passes: u32,
    pub offscreen_passes: u32,
    pub direct_present: bool,
    pub transient_bytes: u64,
    pub synchronous_pipeline_compiles: u32,
    pub occluded_siblings: u32,
}

pub(crate) struct Scene<'a> {
    pub tree: &'a LayerTree,
    pub sampled_layers: &'a HashMap<LayerId, engine::scene::Layer>,
    pub pictures: &'a HashMap<PictureId, Arc<CompiledPicture>>,
    pub fonts: &'a FontLibrary,
    pub textures: &'a TextureStore,
    pub shader_uniforms: &'a crate::runtime_effect::ShaderUniformOverrides,
    pub scale: f32,
    pub slice: Option<&'a HashSet<LayerId>>,
}

impl<'a> Scene<'a> {
    fn layer(&self, id: LayerId) -> Option<&'a engine::scene::Layer> {
        self.sampled_layers
            .get(&id)
            .or_else(|| self.tree.layers.get(id.0 as usize))
    }
}

#[derive(Clone)]
struct Image {
    view: Attachment,
    storage_view: Option<wgpu::TextureView>,
    size: [u32; 2],
    logical: [u32; 2],
    origin: [u32; 2],
    sample_offset: [f32; 2],
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
    material: Option<Arc<pipelines::MaterialCache>>,
}

impl Image {
    fn new(gpu: &Gpu, size: [u32; 2], format: wgpu::TextureFormat, samples: u32) -> Self {
        Self::with_usage(gpu, size, format, samples, Self::usage(samples))
    }

    fn usage(samples: u32) -> wgpu::TextureUsages {
        use wgpu::TextureUsages as U;
        if samples > 1 {
            U::RENDER_ATTACHMENT
        } else {
            U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST
        }
    }

    fn with_usage(
        gpu: &Gpu,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let storage_srgb = format.is_srgb() && usage.contains(wgpu::TextureUsages::STORAGE_BINDING);
        let srgb_view = [format];
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto offscreen"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: if storage_srgb {
                format.remove_srgb_suffix()
            } else {
                format
            },
            usage,
            view_formats: if storage_srgb { &srgb_view } else { &[] },
        });
        let (view, storage_view) = if storage_srgb {
            (
                texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(format),
                    usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
                    ..Default::default()
                }),
                Some(texture.create_view(&wgpu::TextureViewDescriptor {
                    usage: Some(wgpu::TextureUsages::STORAGE_BINDING),
                    ..Default::default()
                })),
            )
        } else {
            (texture.create_view(&Default::default()), None)
        };
        Self {
            view,
            storage_view,
            size,
            logical: size,
            origin: [0; 2],
            sample_offset: [0.0; 2],
            format,
            samples,
            usage,
            material: Some(Arc::default()),
        }
    }

    fn output(view: &wgpu::TextureView, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        Self {
            view: view.clone(),
            storage_view: None,
            size,
            logical: size,
            origin: [0; 2],
            sample_offset: [0.0; 2],
            format,
            samples: 1,
            usage: view.texture().usage(),
            material: Some(Arc::default()),
        }
    }

    fn area(&self) -> [u32; 4] {
        [
            self.origin[0],
            self.origin[1],
            self.logical[0],
            self.logical[1],
        ]
    }

    fn local(&self, area: [u32; 4]) -> [u32; 4] {
        [
            area[0] - self.origin[0],
            area[1] - self.origin[1],
            area[2],
            area[3],
        ]
    }

    fn uv_bounds(&self) -> [f32; 4] {
        let [w, h] = self.size.map(|v| v as f32);
        [
            0.5 / w,
            0.5 / h,
            (self.logical[0] as f32 - 0.5) / w,
            (self.logical[1] as f32 - 0.5) / h,
        ]
    }

    fn bytes(&self) -> u64 {
        let channels = match self.format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => 4,
        };
        u64::from(self.size[0]) * u64::from(self.size[1]) * u64::from(self.samples) * channels
    }
}

impl Compositor {
    pub fn new(gpu: &Gpu, surface_format: Option<wgpu::TextureFormat>) -> Result<Self, Status> {
        let white = Image::new(gpu, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1);
        let white_view = white.view;
        gpu.queue.write_texture(
            white_view.texture().as_image_copy(),
            &[255; 4],
            Default::default(),
            extent([1, 1]),
        );
        let mut pipelines = PipelineCache::new(gpu);
        let specialize_paint = true;
        if std::env::var("IMPASTO_PREWARM").as_deref() != Ok("0") {
            let format = surface_format.unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
            pipelines.prewarm(gpu, format, specialize_paint, surface_format.is_some())?;
        }
        let linear = pipelines::linear_sampler(gpu);
        let white_mask_group = pipelines::bind_group(
            gpu,
            &pipelines.layouts.material,
            &[
                wgpu::BindingResource::TextureView(&white_view),
                wgpu::BindingResource::Sampler(&linear),
            ],
        );
        Ok(Self {
            atlas: Atlas::new(gpu),
            glyph_wake: None,
            glyph_prefetch_deadline: None,
            has_submitted: false,
            pipelines,
            warm_formats: Vec::new(),
            pool: AttachmentPool::default(),
            white: white_view,
            white_mask_group,
            linear,
            nearest: pipelines::nearest_sampler(gpu),
            render_stats: RenderStats::default(),
            arena: FrameArena::new(gpu),
            draw_lists: Vec::new(),
            last_viewport: None,
            runtime_effects: crate::runtime_effect::RuntimeEffects::default(),
            runtime_texture_groups: HashMap::new(),
            effect_seconds: 0.0,
            image_groups: HashMap::new(),
            composites: HashMap::new(),
            masks: HashMap::new(),
            offscreen_live_bytes: 0,
            frame_active: false,
            trim_pending: false,
            specialize_paint,
            compute_blur: [None, None],
            blur_groups: HashMap::new(),
            transient: gpu.adapter.get_info().transient_saves_memory == Some(true),
        })
    }

    pub fn prewarm_surface(&mut self, gpu: &Gpu, format: wgpu::TextureFormat) {
        if std::env::var("IMPASTO_PREWARM").as_deref() == Ok("0") {
            return;
        }
        if let Err(error) = self
            .pipelines
            .prewarm(gpu, format, self.specialize_paint, true)
        {
            tracing::debug!(target: "impasto::startup", ?error, "surface pipeline prewarm failed");
            return;
        }
        for format in [format, wgpu::TextureFormat::Rgba8UnormSrgb] {
            if !self.warm_formats.contains(&format) {
                self.warm_formats.push(format);
            }
        }
    }

    pub(crate) fn note_viewport(&mut self, pixels: Option<[u32; 2]>) {
        if self.last_viewport != pixels || pixels.is_none() {
            self.last_viewport = pixels;
            self.atlas.set_prepare_owner(None);
            self.glyph_prefetch_deadline = None;
            self.has_submitted = false;
        }
    }

    pub(crate) fn collect_resources(&mut self) -> bool {
        self.atlas.collect_prepared()
    }

    pub fn begin_frame(
        &mut self,
        wake: Option<&Arc<dyn Fn() + Send + Sync>>,
        deadline: Option<std::time::Instant>,
    ) {
        let wake = wake.filter(|_| crate::prepare::pool().is_some());
        self.glyph_wake = wake.cloned();
        let prefetched = self.glyph_prefetch_deadline.take();
        let budget = if self.has_submitted {
            glyph_prepare::PREPARE_BUDGET
        } else {
            glyph_prepare::COLD_MASK_BUDGET
        };
        let initial = std::time::Instant::now() + budget;
        self.atlas.set_deadline(wake.map(|_| {
            deadline
                .into_iter()
                .chain(prefetched)
                .fold(initial, std::cmp::min)
        }));
        // A new frame also closes a discarded encoding transaction.
        if self.trim_pending {
            self.trim_resources();
        }
        self.frame_active = true;
        self.atlas.begin_frame();
        self.arena.reset();
        self.recycle_masks();
        self.offscreen_live_bytes = 0;
        self.render_stats = RenderStats::default();
        self.pipelines.reset_frame_compiles();
    }

    pub fn flush(
        &mut self,
        gpu: &Gpu,
    ) -> Result<(AtlasStats, Option<wgpu::CommandBuffer>), Status> {
        let mut prelude = None;
        self.arena.flush(gpu, &mut prelude)?;
        self.atlas.flush(gpu, &mut prelude);
        self.recycle_masks();
        let pooled = |view: &wgpu::TextureView| {
            *view == self.white
                || self
                    .pool
                    .iter()
                    .any(|image| image.view == *view || image.storage_view.as_ref() == Some(view))
        };
        self.composites
            .retain(|(backdrop, mask), _| pooled(backdrop) && pooled(mask));
        self.runtime_texture_groups.retain(|views, group| {
            std::mem::take(&mut group.used)
                && views
                    .iter()
                    .zip(group.backdrops)
                    .all(|(view, backdrop)| !backdrop || pooled(view))
        });
        self.blur_groups.retain(|(source, target, buffer), _| {
            pooled(source) && pooled(target) && self.arena.owns_uniform_buffer(buffer)
        });
        self.render_stats.pooled_texture_bytes = self.pool.bytes;
        self.render_stats.buffers_created += self.arena.buffers_created;
        self.render_stats.mesh_upload_bytes += self.arena.mesh_upload_bytes;
        self.render_stats.bind_groups_created += self.arena.bind_groups_created;
        let [vertex_bytes, uniform_bytes, stop_bytes, extra_bytes] = self.arena.bytes_uploaded();
        self.render_stats.vertex_bytes += vertex_bytes;
        self.render_stats.uniform_bytes += uniform_bytes;
        self.render_stats.stop_bytes += stop_bytes;
        self.render_stats.extra_bytes += extra_bytes;
        self.render_stats.synchronous_pipeline_compiles = self.pipelines.frame_compiles();
        let atlas_stats = self.atlas.take_stats();
        self.render_stats.atlas_upload_bytes = atlas_stats.upload_bytes;
        self.render_stats.copy_pixels += atlas_stats.copy_pixels;
        self.frame_active = false;
        if self.trim_pending {
            self.trim_resources();
        }
        Ok((atlas_stats, prelude.map(wgpu::CommandEncoder::finish)))
    }

    pub fn submitted(&mut self, gpu: &Gpu) {
        self.has_submitted = true;
        self.arena.submitted(gpu);
        // Drive StagingBelt recall callbacks even without strict frame validation.
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        if !self.warm_formats.is_empty() {
            // On native drivers pipeline creation may serialize across threads.
            // Start background effects when the first frame is already queued.
            self.pipelines.prewarm_effects(gpu, &self.warm_formats);
            self.warm_formats.clear();
        }
    }

    pub fn evict_images(&mut self, evictions: impl Iterator<Item = crate::textures::EvictedImage>) {
        let views: Vec<_> = evictions.map(|image| image.view).collect();
        if views.is_empty() {
            return;
        }
        self.image_groups
            .retain(|(group, _), _| group.iter().all(|view| !views.contains(view)));
        self.runtime_texture_groups
            .retain(|group, _| group.iter().all(|view| !views.contains(view)));
    }

    fn finish_glyphs(&mut self, fonts: &FontLibrary) -> Result<(), Status> {
        if let Some(wake) = &self.glyph_wake
            && self.atlas.defer_remaining(fonts, wake)
        {
            return Err(Status::ResourcesPending);
        }
        Ok(())
    }

    pub fn render_stats(&self) -> RenderStats {
        self.render_stats
    }

    pub fn take_effect_failures(&mut self) -> Vec<crate::EffectFailure> {
        self.runtime_effects.take_failures()
    }

    pub fn trim(&mut self) {
        if self.frame_active {
            // Earlier UIKit slices can still reference data waiting for flush.
            self.trim_pending = true;
        } else {
            self.trim_resources();
        }
    }

    pub fn discard_frame(&mut self) {
        self.frame_active = false;
        if self.trim_pending {
            self.trim_resources();
        }
    }

    fn trim_resources(&mut self) {
        self.trim_pending = false;
        self.atlas.trim();
        self.pool.clear();
        self.image_groups.clear();
        self.composites.clear();
        self.runtime_texture_groups.clear();
        self.blur_groups.clear();
        self.arena.trim();
        self.masks.clear();
    }

    fn recycle_masks(&mut self) {
        for (_, image) in self.masks.drain() {
            self.pool.recycle(image);
        }
    }
}

#[derive(Clone, Copy)]
enum ImageSource<'a> {
    External(&'a crate::GpuImage),
    Pooled(&'a Image),
}

impl<'a> ImageSource<'a> {
    fn material(self, frame: &mut Frame<'_>, nearest: bool) -> PassBinding {
        let image = match self {
            Self::External(image) => image,
            Self::Pooled(image) => return frame.sample_attachment(image, nearest),
        };
        let (view, cache) = (image.view(), image.material.as_ref());
        let compositor = &mut frame.compositor;
        let sampler = if nearest {
            &compositor.nearest
        } else {
            &compositor.linear
        };
        let (group, created) = match (self.nv12(), &compositor.pipelines.layouts.external_image) {
            (Some(planes), Some(layout)) => {
                cache.nv12(frame.gpu, layout, view, planes, sampler, nearest)
            }
            _ => cache.get(
                frame.gpu,
                &compositor.pipelines.layouts.material,
                view,
                sampler,
                nearest,
            ),
        };
        compositor.render_stats.bind_groups_created += u32::from(created);
        PassBinding::Ready(group)
    }

    fn nv12(self) -> Option<&'a crate::textures::Nv12Planes> {
        match self {
            Self::External(image) => image.nv12(),
            Self::Pooled(_) => None,
        }
    }

    fn premultiplied_srgb(self) -> bool {
        matches!(self, Self::External(image) if image.premultiplied_srgb)
    }
}

struct PictureReplay<'a> {
    layer: LayerId,
    picture: &'a CompiledPicture,
    origin: Affine,
    group_bounds: Option<Vec<PictureBounds>>,
}

impl PictureReplay<'_> {
    fn group(&self, id: usize) -> Result<Group, Status> {
        let mut group = *self.picture.groups.get(id).ok_or(Status::BadScene)?;
        group.content = self
            .group_bounds
            .as_ref()
            .map_or_else(|| group.content.transform(self.origin), |bounds| bounds[id]);
        Ok(group)
    }
}

struct Frame<'a> {
    gpu: &'a Gpu,
    scene: Scene<'a>,
    compositor: &'a mut Compositor,
    command: wgpu::CommandEncoder,
    pending: Option<DrawPass>,
    // Keep the main target open logically while encoding its input textures.
    // A target read or an incompatible attachment ends the current segment.
    main_pass: Option<DrawPass>,
    main_inputs: PassInputs,
    late_acquire: bool,
    deferred_root: Option<DeferredRootPass>,
    clears: HashMap<Attachment, wgpu::Color>,
    root: Attachment,
    main: Attachment,
    capture: Option<(&'a mut Capture, Option<usize>)>,
    size: [u32; 2],
    working: wgpu::TextureFormat,
    straight_alpha: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum PassBinding {
    Ready(wgpu::BindGroup),
    Sample(ImageBinding),
}

#[derive(Clone)]
struct ImageBinding {
    view: Attachment,
    cache: Option<Arc<pipelines::MaterialCache>>,
    nearest: bool,
}

impl PartialEq for ImageBinding {
    fn eq(&self, other: &Self) -> bool {
        self.view == other.view && self.nearest == other.nearest
    }
}

impl Eq for ImageBinding {}

impl std::hash::Hash for ImageBinding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.view.hash(state);
        self.nearest.hash(state);
    }
}

impl From<wgpu::BindGroup> for PassBinding {
    fn from(group: wgpu::BindGroup) -> Self {
        Self::Ready(group)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum DrawPipeline {
    Standard(PipelineId),
    Runtime(wgpu::RenderPipeline),
}

impl DrawPipeline {
    fn get<'a>(&'a self, pipelines: &'a PipelineCache) -> &'a wgpu::RenderPipeline {
        match self {
            Self::Standard(id) => pipelines.pipeline(*id),
            Self::Runtime(pipeline) => pipeline,
        }
    }
}

#[derive(Clone)]
struct CompositeDraw {
    pipeline: PipelineId,
    globals: DynamicGroup,
    source: wgpu::BindGroup,
    composite: Option<wgpu::BindGroup>,
    effect_values: Option<(u32, DynamicGroup)>,
    immediates: Option<ParamsUniform>,
    viewport: [u32; 4],
}

impl CompositeDraw {
    fn encode(&self, pass: &mut wgpu::RenderPass<'_>, pipelines: &PipelineCache) {
        let [x, y, w, h] = self.viewport.map(|value| value as f32);
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(pipelines.pipeline(self.pipeline));
        pass.set_bind_group(0, &self.globals.group, self.globals.offsets());
        pass.set_bind_group(1, &self.source, &[]);
        if let Some(params) = &self.immediates {
            pass.set_immediates(0, bytemuck::bytes_of(params));
        }
        if let Some(composite) = &self.composite {
            pass.set_bind_group(2, composite, &[]);
        }
        if let Some((slot, values)) = &self.effect_values {
            pass.set_bind_group(*slot, &values.group, values.offsets());
        }
        pass.draw(0..6, 0..1);
    }
}

#[derive(Clone, Copy)]
struct GlyphTransform {
    matrix: Affine,
    scale: f32,
    pixels: f32,
    aligned: bool,
    phase_x: u32,
    buckets: u32,
}

impl GlyphTransform {
    fn new(mut matrix: Affine, scale: f32) -> Self {
        let [a, b, c, d, x, y] = matrix;
        let aligned = b.abs() < 0.0001
            && c.abs() < 0.0001
            && (a - 1.0).abs() <= 0.001
            && (a - d).abs() < 0.0001;
        let buckets = if scale < 2.0 {
            4
        } else if scale < 2.75 {
            2
        } else {
            1
        };
        let mut phase_x = 0;
        if aligned {
            let steps = buckets as f32;
            let snapped_x = (x * scale * steps).round() / steps;
            phase_x = crate::atlas::horizontal_bucket(snapped_x, buckets);
            matrix[4] = snapped_x / scale;
            matrix[5] = (y * scale + 0.5).floor() / scale;
        }
        Self {
            matrix,
            scale,
            pixels: scale * a.hypot(b).max(c.hypot(d)).max(0.001),
            aligned,
            phase_x,
            buckets,
        }
    }

    fn position(self, glyph: &compiler::GlyphDraw) -> Option<SubpixelPosition> {
        let [a, b, c, d, ..] = self.matrix;
        // Relative positions keep the same fractional bits across integer translations.
        self.aligned.then(|| SubpixelPosition {
            point: [
                (a * glyph.x + c * glyph.y) * self.scale
                    + self.phase_x as f32 / crate::atlas::SUBPIXEL_BUCKETS as f32,
                (b * glyph.x + d * glyph.y) * self.scale,
            ],
            buckets: self.buckets,
        })
    }

    fn local_rect(
        self,
        glyph: &compiler::GlyphDraw,
        position: Option<SubpixelPosition>,
        placement: crate::atlas::Placement,
    ) -> [f32; 4] {
        let [a, _, _, d, ..] = self.matrix;
        let [dx, dy] = position.map_or([0.0; 2], |position| {
            let [x, y] = position.point;
            [
                x.rem_euclid(1.0) / (self.scale * a),
                (y - (y + 0.5).floor()) / (self.scale * d),
            ]
        });
        [
            glyph.x - dx + placement.left as f32 / self.pixels,
            glyph.y - dy - placement.top as f32 / self.pixels,
            placement.rect.w as f32 / self.pixels,
            placement.rect.h as f32 / self.pixels,
        ]
    }
}

#[derive(Clone, Default)]
struct Coverage {
    scissor: Option<[u32; 4]>,
    mask: Option<Image>,
    empty: bool,
    boxes: [[f32; 4]; 4],
    radii: [[f32; 4]; 4],
    curves: [f32; 4],
    count: usize,
}

impl Coverage {
    fn view(&self, target: &Image, scale: f32) -> ViewUniform {
        ViewUniform {
            meta: [
                scale,
                self.count as f32,
                f32::from(self.mask.is_some()),
                0.0,
            ],
            target_info: [target.origin[0] as f32, target.origin[1] as f32, 0.0, 0.0],
            mask_rect: self.mask.as_ref().map_or([0.0; 4], |mask| {
                [
                    mask.origin[0] as f32,
                    mask.origin[1] as f32,
                    1.0 / mask.size[0] as f32,
                    1.0 / mask.size[1] as f32,
                ]
            }),
            clip_boxes: self.boxes,
            clip_radii: self.radii,
            clip_curves: self.curves,
            ..ViewUniform::zeroed()
        }
    }
}

impl Frame<'_> {
    fn glyphs<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        glyphs: impl IntoIterator<Item = &'a compiler::GlyphDraw>,
    ) -> Result<(), Status> {
        let mut batch = Vec::new();
        let mut key = AtlasBinding::default();
        let empty = self.stops(&[])?;
        let (mut active, _) = self.paint(empty, None)?;
        let atlas_scale = self.compositor.atlas.inverse_page_size();
        let geometry = GlyphTransform::new(transform, self.scene.scale);
        let view = self.geometry_view(target, geometry.matrix, clip, atlas_scale);
        let mut last_gradient: Option<(&batch::PackedGradient, PaintPage, u32)> = None;
        for glyph in glyphs {
            if self
                .area(
                    glyph
                        .bounds()
                        .transform(geometry.matrix)
                        .expand(1.0 / self.scene.scale),
                    target,
                )
                .is_none()
            {
                continue;
            }
            let (mut vertex, binding, placement) = if let Some(rect) = glyph.decoration {
                (
                    GlyphVertex {
                        rect,
                        uv: [0; 4],
                        color: glyph.color,
                        flags_and_layer: batch::GLYPH_DECORATION,
                        paint: u32::MAX,
                    },
                    key,
                    None,
                )
            } else {
                let Some((vertex, binding, placement)) = self.glyph_quad(glyph, geometry)? else {
                    continue;
                };
                (vertex, binding, Some(placement))
            };
            let color = placement.is_some_and(|placement| placement.color);
            let (page, index) = if let Some(gradient) = glyph.gradient.as_deref().filter(|_| !color)
            {
                vertex.flags_and_layer |= batch::GLYPH_GRADIENT;
                match last_gradient {
                    Some((last, page, index))
                        if std::ptr::eq(last, gradient) && page.same_bindings(active) =>
                    {
                        (page, index)
                    }
                    _ => {
                        let inline = gradient.inline_pair();
                        let pairs = if inline.is_some() {
                            &[][..]
                        } else {
                            gradient.stops.as_slice()
                        };
                        let stops = self.stops(pairs)?;
                        let [start, end] = inline.unwrap_or([
                            [0.0; 4],
                            [gradient.radial_radii[0], gradient.radial_radii[1], 0.0, 0.0],
                        ]);
                        let (page, index) = self.paint(
                            stops,
                            Some(PaintExtra {
                                geometry: gradient.a,
                                gradient: [
                                    inline.map_or(gradient.kind, |_| batch::INLINE_LINEAR_GRADIENT)
                                        as f32,
                                    FrameArena::stop_index(stops),
                                    (pairs.len() / 2) as f32,
                                    0.0,
                                ],
                                shadow: end,
                                stroke: start,
                                corners: [0.0; 4],
                            }),
                        )?;
                        last_gradient = Some((gradient, page, index));
                        (page, index)
                    }
                }
            } else {
                // Solid glyphs can share any currently bound page.
                (active, u32::MAX)
            };
            if key != binding || !active.same_bindings(page) {
                self.glyph_batch(target, clip, view, active, &batch, key)?;
                batch.clear();
            }
            key = binding;
            active = page;
            vertex.paint = index;
            batch.push(vertex);
        }
        self.glyph_batch(target, clip, view, active, &batch, key)
    }

    fn glyph_quad(
        &mut self,
        glyph: &compiler::GlyphDraw,
        geometry: GlyphTransform,
    ) -> Result<Option<(GlyphVertex, AtlasBinding, crate::atlas::Placement)>, Status> {
        let position = geometry.position(glyph);
        let size = glyph.size * geometry.pixels;
        let Some(placement) =
            self.compositor
                .atlas
                .glyph(self.gpu, self.scene.fonts, glyph, size, position)
        else {
            return Ok(None);
        };
        if placement.rect.w == 0 || placement.rect.h == 0 {
            return Ok(None);
        }
        let (binding, layer) = self.compositor.atlas.binding(placement);
        let mut flags = if placement.color {
            batch::GLYPH_COLOR
        } else {
            0
        };
        if !placement.color && size <= 14.0 {
            flags |= batch::GLYPH_GAMMA;
        }
        if matches!(binding, AtlasBinding::Temporary { .. }) {
            flags |= batch::GLYPH_TEMPORARY_PAGE;
        }
        Ok(Some((
            GlyphVertex {
                rect: geometry.local_rect(glyph, position, placement),
                uv: self
                    .compositor
                    .atlas
                    .texels(placement)
                    .ok_or(Status::BadScene)?,
                color: glyph.color,
                flags_and_layer: flags | (layer << 8),
                paint: u32::MAX,
            },
            binding,
            placement,
        )))
    }

    fn glyph_batch(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: ViewUniform,
        page: PaintPage,
        glyphs: &[GlyphVertex],
        key: AtlasBinding,
    ) -> Result<(), Status> {
        if glyphs.is_empty() {
            return Ok(());
        }
        let (material, created) = self.compositor.atlas.material(
            self.gpu,
            &self.compositor.pipelines.layouts.atlas,
            &self.compositor.linear,
            key,
        );
        self.compositor.render_stats.bind_groups_created += u32::from(created);
        self.read_mask(target, clip);
        let mask = self.mask_group(clip);
        let kind = if self.compositor.specialize_paint
            && glyphs.iter().all(|glyph| glyph.flags_and_layer & 7 == 0)
        {
            PipelineKind::TextMask
        } else if clip.count == 0 && clip.mask.is_none() {
            PipelineKind::TextUnclipped
        } else {
            PipelineKind::Text
        };
        self.record_instances(
            target,
            view,
            clip.scissor,
            kind,
            BlendKey::SrcOver,
            if matches!(kind, PipelineKind::TextMask) {
                Extra::None
            } else {
                Extra::Paint(page)
            },
            material,
            Some(mask),
            glyphs,
            6,
        )
    }

    fn uniform<T: bytemuck::Pod>(&mut self, value: &T) -> Result<Address, Status> {
        self.compositor.arena.uniform(self.gpu, value)
    }

    fn effect_values(&mut self, values: &[[f32; 4]; 5]) -> Result<DynamicGroup, Status> {
        self.compositor
            .arena
            .effect_values(self.gpu, &self.compositor.pipelines.layouts, values)
    }

    fn transform(&mut self, target: &Image, t: Affine, clip: &Coverage) -> Result<Address, Status> {
        self.geometry_uniform(target, t, clip, 0.0)
    }

    fn geometry_uniform(
        &mut self,
        target: &Image,
        t: Affine,
        clip: &Coverage,
        value: f32,
    ) -> Result<Address, Status> {
        let view = self.geometry_view(target, t, clip, value);
        self.uniform(&view)
    }

    fn geometry_view(&self, target: &Image, t: Affine, clip: &Coverage, value: f32) -> ViewUniform {
        let [w, h] = target.size.map(|v| v as f32);
        let sx = 2.0 * self.scene.scale / w;
        let sy = -2.0 * self.scene.scale / h;
        let [x, y] = target.origin.map(|v| v as f32 / self.scene.scale);
        let scale = compiler::primitive_scale(t, self.scene.scale);
        let [a, b, c, d, ..] = t;
        let determinant = (a * d - b * c).abs();
        let aa = if a == d && b == -c {
            scale.recip()
        } else {
            (d.hypot(b) + c.hypot(a)) / (2.0 * determinant * self.scene.scale)
        };
        let mut view = ViewUniform {
            transform: [
                [t[0] * sx, t[1] * sy, 0.0, 0.0],
                [t[2] * sx, t[3] * sy, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [(t[4] - x) * sx - 1.0, (t[5] - y) * sy + 1.0, 0.0, 1.0],
            ],
            ..clip.view(target, scale)
        };
        view.meta[3] = value;
        if aa.is_finite() && determinant > 1e-6 {
            view.target_info[2] = aa.max(0.5e-6);
        }
        view
    }

    fn stops(&mut self, stops: &[[f32; 4]]) -> Result<Address, Status> {
        self.compositor.arena.stops(self.gpu, stops)
    }

    fn paint(
        &mut self,
        stops: Address,
        paint: Option<PaintExtra>,
    ) -> Result<(PaintPage, u32), Status> {
        self.compositor.arena.paint(self.gpu, stops, paint)
    }

    fn globals(&mut self, view: Address, extra: Extra) -> DynamicGroup {
        self.compositor
            .arena
            .group(self.gpu, &self.compositor.pipelines.layouts, view, extra)
    }

    fn mask_group(&self, coverage: &Coverage) -> PassBinding {
        match &coverage.mask {
            Some(mask) => self.sample_attachment(mask, false),
            None => PassBinding::Ready(self.compositor.white_mask_group.clone()),
        }
    }

    fn sample_attachment(&self, image: &Image, nearest: bool) -> PassBinding {
        PassBinding::Sample(ImageBinding {
            view: image.view.clone(),
            cache: image.material.clone(),
            nearest,
        })
    }

    fn composite_group(
        &mut self,
        backdrop: Option<wgpu::TextureView>,
        mask: Option<wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let compositor = &mut self.compositor;
        let key = (
            backdrop.unwrap_or_else(|| compositor.white.clone()),
            mask.unwrap_or_else(|| compositor.white.clone()),
        );
        if compositor.composites.len() >= 128 && !compositor.composites.contains_key(&key) {
            compositor.composites.clear();
        }
        compositor
            .composites
            .entry(key)
            .or_insert_with_key(|(backdrop, mask)| {
                compositor.render_stats.bind_groups_created += 1;
                pipelines::bind_group(
                    self.gpu,
                    &compositor.pipelines.layouts.composite,
                    &[
                        wgpu::BindingResource::TextureView(backdrop),
                        wgpu::BindingResource::Sampler(&compositor.linear),
                        wgpu::BindingResource::TextureView(mask),
                    ],
                )
            })
            .clone()
    }
}

#[derive(Default)]
struct AttachmentPool {
    images: Vec<Image>,
    bytes: u64,
}

impl AttachmentPool {
    fn take(
        &mut self,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Option<Image> {
        let index = self.images.iter().rposition(|image| {
            image.size == size
                && image.format == format
                && image.samples == samples
                && image.usage == usage
        })?;
        let image = self.images.remove(index);
        self.bytes -= image.bytes();
        Some(image)
    }

    fn recycle(&mut self, image: Image) {
        let bytes = image.bytes();
        if bytes > POOL_BYTES {
            return;
        }
        while self.images.len() >= POOL_LIMIT || self.bytes + bytes > POOL_BYTES {
            self.bytes -= self.images.remove(0).bytes();
        }
        self.images.push(image);
        self.bytes += bytes;
    }

    fn iter(&self) -> impl Iterator<Item = &Image> {
        self.images.iter()
    }

    fn clear(&mut self) {
        self.images.clear();
        self.bytes = 0;
    }
}

fn single(format: wgpu::TextureFormat) -> Target {
    Target { format, samples: 1 }
}

pub(crate) const fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    }
}

fn set_rect_gradient(rect: &mut RectInstance, gradient: &batch::PackedGradient) {
    rect.flags |= batch::RECT_GRADIENT;
    rect.gradient_a = gradient.a;
    rect.shadow[..2].copy_from_slice(&gradient.radial_radii);
    rect.gradient_b = [
        gradient.kind as f32,
        0.0,
        (gradient.stops.len() / 2) as f32,
        1.0,
    ];
}

fn draw_count(len: usize) -> Result<u32, Status> {
    u32::try_from(len).map_err(|_| Status::BadScene)
}

const fn translation(x: f32, y: f32) -> Affine {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the pixel halo saturates before clipping to the target"
)]
fn expand_pixels(area: [u32; 4], radius: f32, limit: [u32; 4]) -> [u32; 4] {
    let halo = (3.0 * radius).ceil() as u32;
    let x = area[0].saturating_sub(halo).max(limit[0]);
    let y = area[1].saturating_sub(halo).max(limit[1]);
    let right = (area[0] + area[2])
        .saturating_add(halo)
        .min(limit[0] + limit[2]);
    let bottom = (area[1] + area[3])
        .saturating_add(halo)
        .min(limit[1] + limit[3]);
    [x, y, right - x, bottom - y]
}

fn intersect_pixels(a: [u32; 4], b: [u32; 4]) -> Option<[u32; 4]> {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    let w = right.checked_sub(x)?;
    let h = bottom.checked_sub(y)?;
    (w > 0 && h > 0).then_some([x, y, w, h])
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    [
        x,
        y,
        ((a[0] + a[2]).min(b[0] + b[2]) - x).max(0.0),
        ((a[1] + a[3]).min(b[1] + b[3]) - y).max(0.0),
    ]
}

fn set_scissor(pass: &mut wgpu::RenderPass<'_>, bounds: Option<[u32; 4]>) {
    if let Some([x, y, w, h]) = bounds {
        pass.set_scissor_rect(x, y, w, h);
    }
}
