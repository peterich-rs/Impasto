//! Ordered scene replay, clipping, and offscreen composition.

mod backdrop_cache;
mod canvas_pass;
mod depth;
mod glyph_prepare;
mod image_array;
mod instances;
mod resident;
mod retained;
mod small_images;
mod subtree_cache;
mod transition_snapshot;

pub use subtree_cache::SubtreeCacheStats;
mod runtime_effect;

use crate::profiler::{Capture, GpuPassKind, GpuPassSource};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use engine::scene::{LayerId, LayerKind, LayerTree, PictureId};

use bytemuck::Zeroable;

use crate::arena::{Address, DynamicGroup, Extra, FrameArena, PaintPage, VertexSlice};
use crate::atlas::{Atlas, AtlasBinding, AtlasStats, SubpixelPosition};
use crate::batch::{self, GlyphVertex, ImageInstance, Mesh, PaintExtra, RectInstance, RectVertex};
use crate::batcher::{Bounds, OverlapRows, SpatialBatcher};
use crate::compiler::{
    self, Affine, Clip, ClipShape, CompiledPicture, Draw, Effect, Group, IDENTITY, PictureBounds,
    Scope,
};
use crate::compute_blur::{BlurParams, ComputeBlur};
use crate::gpu::Gpu;
use crate::gradient_lut::PreparedRamp;
use crate::graph::{AttachmentLifetimes, RenderGraph, SurfaceGraph};
use crate::msdf::MsdfAtlas;
use crate::outline::{Outlines, REFERENCE_PPEM};
use crate::pipelines::{
    self, BlendKey, ParamsUniform, PipelineCache, PipelineKind, Target, ViewUniform,
};
use crate::resident::ResidentPictures;
use crate::{FontLibrary, Status, textures::TextureStore};
use canvas_pass::CanvasPass;
use depth::DepthPass;
pub(crate) use glyph_prepare::GlyphScene;
use retained::RetainedRoot;
use subtree_cache::{CacheCandidate, CacheSlot, MIN_RESAMPLED_BLUR, SubtreeCache};

const POOL_LIMIT: usize = 48;
const MASK_LIMIT: usize = 16;
const POOL_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct Compositor {
    atlas: Atlas,
    outlines: Outlines,
    msdf: MsdfAtlas,
    use_msdf: bool,
    pipelines: PipelineCache,
    warm_formats: Vec<wgpu::TextureFormat>,
    pool: AttachmentPool,
    white: wgpu::TextureView,
    white_mask_group: wgpu::BindGroup,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    render_stats: RenderStats,
    arena: FrameArena,
    resident: ResidentPictures,
    subtrees: SubtreeCache,
    backdrops: backdrop_cache::BackdropCache,
    transition_snapshots: transition_snapshot::TransitionSnapshots,
    retained: Option<RetainedRoot>,
    retain_offscreen: Option<bool>,
    last_viewport: Option<[u32; 2]>,
    runtime_effects: crate::runtime_effect::RuntimeEffects,
    runtime_texture_groups: HashMap<
        [wgpu::TextureView; engine::scene::MAX_SHADER_TEXTURES],
        runtime_effect::TextureGroup,
    >,
    pub(super) effect_seconds: f32,
    small_images: small_images::SmallImages,
    image_groups: HashMap<(Vec<wgpu::TextureView>, bool), wgpu::BindGroup>,
    composites: HashMap<(wgpu::TextureView, wgpu::TextureView), wgpu::BindGroup>,
    masks: HashMap<MaskKey, Image>,
    offscreen_live_bytes: u64,
    frame_active: bool,
    trim_pending: bool,
    specialize_paint: bool,
    compute_blur: [Option<ComputeBlur>; 2],
    blur_groups: HashMap<(wgpu::TextureView, wgpu::TextureView, wgpu::Buffer), wgpu::BindGroup>,
    spatial_batch: bool,
    prefer_spatial: bool,
    render_graph: bool,
    early_z: bool,
    adaptive_early_z: bool,
    canvas_stencil: bool,
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
    pub gradient_lut_bytes: u64,
    pub gradient_lut_hits: u32,
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
    pub early_z_instances: u32,
    pub stencil_draws: u32,
    pub stencil_batches: u32,
    pub spatial_merges: u32,
    pub msdf_misses: u32,
    pub synchronous_pipeline_compiles: u32,
    pub subtree_cache: SubtreeCacheStats,
    pub partial_redraw: bool,
    pub backdrop_cache_hits: u32,
    pub occluded_siblings: u32,
    pub transition_snapshot_hits: u32,
    pub transition_snapshot_bakes: u32,
}

pub(crate) struct Scene<'a> {
    pub tree: &'a LayerTree,
    pub pictures: &'a HashMap<PictureId, Arc<CompiledPicture>>,
    pub fonts: &'a FontLibrary,
    pub textures: &'a TextureStore,
    pub shader_uniforms: &'a crate::runtime_effect::ShaderUniformOverrides,
    pub scale: f32,
    pub slice: Option<&'a HashSet<LayerId>>,
    pub damage: crate::DamageRect,
    pub has_glyphs: bool,
    pub revision: u64,
    pub scene_revision: u64,
    pub previous_revision: u64,
}

#[derive(Clone)]
struct Image {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    storage_view: Option<wgpu::TextureView>,
    size: [u32; 2],
    logical: [u32; 2],
    origin: [u32; 2],
    sample_offset: [f32; 2],
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
    material: Arc<pipelines::MaterialCache>,
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
            texture,
            view,
            storage_view,
            size,
            logical: size,
            origin: [0; 2],
            sample_offset: [0.0; 2],
            format,
            samples,
            usage,
            material: Arc::default(),
        }
    }

    fn output(view: &wgpu::TextureView, format: wgpu::TextureFormat, size: [u32; 2]) -> Self {
        Self {
            texture: view.texture().clone(),
            view: view.clone(),
            storage_view: None,
            size,
            logical: size,
            origin: [0; 2],
            sample_offset: [0.0; 2],
            format,
            samples: 1,
            usage: view.texture().usage(),
            material: Arc::default(),
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputFormat {
    pub format: wgpu::TextureFormat,
    space: wgpu::SurfaceColorSpace,
    straight_alpha: bool,
    window: bool,
}

impl OutputFormat {
    pub fn surface(config: &wgpu::SurfaceConfiguration) -> Self {
        Self {
            format: config.format,
            space: config.color_space,
            straight_alpha: config.alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied,
            window: true,
        }
    }
}

impl From<wgpu::TextureFormat> for OutputFormat {
    fn from(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            straight_alpha: false,
            window: false,
            space: if format == wgpu::TextureFormat::Rgba16Float {
                wgpu::SurfaceColorSpace::ExtendedSrgbLinear
            } else {
                wgpu::SurfaceColorSpace::Srgb
            },
        }
    }
}

impl Compositor {
    pub fn new(gpu: &Gpu, surface_format: Option<wgpu::TextureFormat>) -> Result<Self, Status> {
        let white = Image::new(gpu, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1);
        gpu.queue.write_texture(
            white.texture.as_image_copy(),
            &[255; 4],
            Default::default(),
            extent([1, 1]),
        );
        let mut pipelines = PipelineCache::new(gpu);
        let specialize_paint = std::env::var("IMPASTO_SPECIALIZE_PAINT").as_deref() != Ok("0");
        let depth_mode = std::env::var("IMPASTO_EARLY_Z");
        if std::env::var("IMPASTO_PREWARM").as_deref() != Ok("0") {
            let format = surface_format.unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb);
            pipelines.prewarm(gpu, format, specialize_paint, surface_format.is_some())?;
        }
        let linear = pipelines::linear_sampler(gpu);
        let white_mask_group = pipelines::bind_group(
            gpu,
            &pipelines.layouts.material,
            &[
                wgpu::BindingResource::TextureView(&white.view),
                wgpu::BindingResource::Sampler(&linear),
            ],
        );
        Ok(Self {
            atlas: Atlas::new(gpu),
            outlines: Outlines::default(),
            msdf: MsdfAtlas::default(),
            use_msdf: std::env::var("IMPASTO_TEXT_MSDF").as_deref() != Ok("0"),
            pipelines,
            warm_formats: Vec::new(),
            pool: AttachmentPool::default(),
            white: white.view,
            white_mask_group,
            linear,
            nearest: pipelines::nearest_sampler(gpu),
            render_stats: RenderStats::default(),
            arena: FrameArena::new(gpu),
            resident: ResidentPictures::default(),
            subtrees: SubtreeCache::new(),
            backdrops: backdrop_cache::BackdropCache::default(),
            transition_snapshots: transition_snapshot::TransitionSnapshots::default(),
            retained: None,
            last_viewport: None,
            retain_offscreen: match std::env::var("IMPASTO_RETAIN_OFFSCREEN").as_deref() {
                Ok("0") => Some(false),
                Ok("1") => Some(true),
                _ => None,
            },
            runtime_effects: crate::runtime_effect::RuntimeEffects::default(),
            runtime_texture_groups: HashMap::new(),
            effect_seconds: 0.0,
            small_images: small_images::SmallImages::default(),
            image_groups: HashMap::new(),
            composites: HashMap::new(),
            masks: HashMap::new(),
            offscreen_live_bytes: 0,
            frame_active: false,
            trim_pending: false,
            specialize_paint,
            compute_blur: [
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureFormat::Rgba16Float,
            ]
            .map(|format| ComputeBlur::new(gpu, format)),
            blur_groups: HashMap::new(),
            render_graph: std::env::var("IMPASTO_RENDER_GRAPH").as_deref() != Ok("0"),
            prefer_spatial: false,
            spatial_batch: std::env::var("IMPASTO_SPATIAL_BATCH").as_deref() != Ok("0"),
            early_z: depth_mode.as_deref() == Ok("1"),
            adaptive_early_z: depth_mode.is_err()
                && gpu.adapter.get_info().transient_saves_memory == Some(true),
            canvas_stencil: std::env::var("IMPASTO_CANVAS_STENCIL").as_deref() != Ok("0"),
            transient: match std::env::var("IMPASTO_TRANSIENT_ATTACHMENTS").as_deref() {
                Ok("0") => false,
                Ok("1") => true,
                _ => gpu.adapter.get_info().transient_saves_memory == Some(true),
            },
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

    #[cfg(test)]
    pub fn disable_residency(&mut self) {
        self.pipelines.resident_pictures = false;
    }

    #[cfg(test)]
    pub fn disable_glyph_outlines(&mut self) {
        self.outlines.disable();
        self.use_msdf = false;
    }

    pub fn begin_frame(&mut self) {
        // A new frame also closes a discarded encoding transaction.
        if self.trim_pending {
            self.trim_resources();
        }
        self.frame_active = true;
        self.atlas.begin_frame();
        self.msdf.begin();
        self.resident.begin_frame();
        self.subtrees.begin();
        self.backdrops.begin();
        self.transition_snapshots.begin();
        self.small_images.begin();
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
        self.arena
            .flush(gpu, &mut prelude, self.resident.uploads())?;
        self.atlas.flush(gpu, &mut prelude);
        self.recycle_masks();
        let pooled = |view: &wgpu::TextureView| {
            *view == self.white
                || self
                    .pool
                    .iter()
                    .any(|image| image.view == *view || image.storage_view.as_ref() == Some(view))
        };
        self.composites.retain(|(backdrop, mask), _| {
            (pooled(backdrop) || self.backdrops.contains(backdrop)) && pooled(mask)
        });
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
        self.render_stats.uniform_bytes += uniform_bytes + self.resident.uniform_bytes();
        self.render_stats.stop_bytes += stop_bytes;
        self.render_stats.extra_bytes += extra_bytes;
        self.render_stats.gradient_lut_bytes = self.arena.lut.upload_bytes;
        self.render_stats.gradient_lut_hits = self.arena.lut.hits;
        self.render_stats.synchronous_pipeline_compiles = self.pipelines.frame_compiles();
        let mut atlas_stats = self.atlas.take_stats();
        let msdf = self.msdf.stats();
        atlas_stats.uploads += msdf.uploads;
        atlas_stats.upload_bytes += msdf.upload_bytes;
        atlas_stats.hits += msdf.hits;
        atlas_stats.misses += msdf.misses;
        self.render_stats.msdf_misses = msdf.misses;
        self.render_stats.atlas_upload_bytes = atlas_stats.upload_bytes;
        self.render_stats.subtree_cache = self.subtrees.stats();
        self.render_stats.copy_pixels += atlas_stats.copy_pixels;
        self.frame_active = false;
        if self.trim_pending {
            self.trim_resources();
        }
        Ok((atlas_stats, prelude.map(wgpu::CommandEncoder::finish)))
    }

    pub fn submitted(&mut self, gpu: &Gpu) {
        // Repeatedly amortizing one draw across nearby pictures is cheaper
        // than promoting each of those pictures to an individual bundle.
        self.prefer_spatial = self.render_stats.spatial_merges >= 8
            && self.render_stats.spatial_merges >= self.render_stats.draws / 4;
        self.arena.submitted(gpu);
        // Drive StagingBelt recall callbacks even without strict frame validation.
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        self.resident.clear_uploads();
        self.subtrees.submitted();
        self.backdrops.submitted();
        self.transition_snapshots.submitted();
        self.small_images.submitted();
        if let Some(retained) = &mut self.retained {
            retained.submitted = true;
        }
        if !self.warm_formats.is_empty() {
            // On native drivers pipeline creation may serialize across threads.
            // Start background effects when the first frame is already queued.
            self.pipelines.prewarm_effects(gpu, &self.warm_formats);
            self.warm_formats.clear();
        }
    }

    pub fn prune_subtrees(&mut self, pictures: &HashMap<PictureId, Arc<CompiledPicture>>) {
        self.subtrees.prune(pictures);
    }

    pub fn evict_images(&mut self, evictions: impl Iterator<Item = crate::textures::EvictedImage>) {
        let (views, revisions): (HashSet<_>, HashSet<_>) =
            evictions.map(|image| (image.view, image.revision)).unzip();
        if views.is_empty() {
            return;
        }
        self.image_groups
            .retain(|(group, _), _| group.iter().all(|view| !views.contains(view)));
        self.runtime_texture_groups
            .retain(|group, _| group.iter().all(|view| !views.contains(view)));
        self.resident.evict_images(&revisions);
    }

    #[cfg(test)]
    pub fn disable_subtree_cache(&mut self) {
        self.subtrees.disable();
    }

    #[cfg(test)]
    pub fn set_retain_offscreen(&mut self, enabled: bool) {
        self.retain_offscreen = Some(enabled);
    }

    #[cfg(test)]
    pub fn set_spatial_batch(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.spatial_batch, enabled)
    }

    #[cfg(test)]
    pub fn force_early_depth(&mut self) -> bool {
        self.early_z = true;
        self.pipelines.layouts.state.is_some()
    }

    #[cfg(test)]
    pub fn set_render_graph(&mut self, enabled: bool) -> bool {
        std::mem::replace(&mut self.render_graph, enabled)
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
        self.outlines = Outlines::default();
        self.msdf = MsdfAtlas::default();
        self.pool.clear();
        self.image_groups.clear();
        self.small_images = small_images::SmallImages::default();
        self.composites.clear();
        self.runtime_texture_groups.clear();
        self.blur_groups.clear();
        self.arena.trim();
        self.resident = ResidentPictures::default();
        self.subtrees.clear();
        self.backdrops = backdrop_cache::BackdropCache::default();
        self.transition_snapshots = transition_snapshot::TransitionSnapshots::default();
        self.retained = None;
        self.masks.clear();
    }

    fn recycle_masks(&mut self) {
        for (_, image) in self.masks.drain() {
            self.pool.recycle(image, self.render_graph);
        }
    }

    pub fn encode(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: &wgpu::TextureView,
        output_format: impl Into<OutputFormat>,
        size: [u32; 2],
        capture: Option<(&mut Capture, Option<usize>)>,
    ) -> Result<wgpu::CommandEncoder, Status> {
        let prepared = self.prepare_output(
            gpu,
            scene,
            Some(output),
            output_format.into(),
            size,
            capture,
        )?;
        self.retained = prepared.retained;
        Ok(prepared.tail)
    }

    pub fn prepare_surface(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output_format: OutputFormat,
        size: [u32; 2],
        capture: Option<&mut Capture>,
    ) -> Result<PreparedOutput, Status> {
        self.prepare_output(
            gpu,
            scene,
            None,
            output_format,
            size,
            capture.map(|capture| (capture, None)),
        )
    }

    fn prepare_output(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: Option<&wgpu::TextureView>,
        output_format: OutputFormat,
        size: [u32; 2],
        capture: Option<(&mut Capture, Option<usize>)>,
    ) -> Result<PreparedOutput, Status> {
        self.last_viewport = Some(size);
        self.pipelines.install_warmed();
        self.subtrees.output(output_format);
        let OutputFormat {
            format,
            space,
            straight_alpha,
            window,
        } = output_format;
        let working = match space {
            wgpu::SurfaceColorSpace::DisplayP3 | wgpu::SurfaceColorSpace::ExtendedSrgbLinear => {
                wgpu::TextureFormat::Rgba16Float
            }
            _ => wgpu::TextureFormat::Rgba8UnormSrgb,
        };
        self.transition_snapshots.prepare(transition_snapshot::Key {
            revision: scene.scene_revision,
            size,
            scale: scene.scale,
            format: working,
        });
        let command = frame_encoder(gpu);
        let p3 = space == wgpu::SurfaceColorSpace::DisplayP3;
        let linear = space == wgpu::SurfaceColorSpace::ExtendedSrgbLinear;
        let direct_candidate =
            !straight_alpha && (linear && format == working || !p3 && format.is_srgb());
        let retain_enabled = self.retain_offscreen.unwrap_or(window);
        let (visible_root, occluded) = opaque_tail(&scene, size);
        self.render_stats.occluded_siblings += occluded;
        let reads_root =
            (direct_candidate || retain_enabled) && scene_reads_root(&scene, Some(visible_root));
        let small_damage =
            scene.damage != crate::DamageRect::Empty && RetainedRoot::is_small(scene.damage, size);
        let matching_history = self
            .retained
            .as_ref()
            .is_some_and(|root| root.output == output_format && root.image.size == size);
        let retain_direct = self.retain_offscreen.is_none()
            && window
            && (small_damage || scene.damage == crate::DamageRect::Empty && matching_history)
            && RetainedRoot::eligible(&scene, size, working);
        let direct = direct_candidate && !reads_root && !retain_direct;
        let dither_output = gpu.dither_output
            && !direct
            && matches!(
                format,
                wgpu::TextureFormat::Bgra8Unorm
                    | wgpu::TextureFormat::Bgra8UnormSrgb
                    | wgpu::TextureFormat::Rgba8Unorm
                    | wgpu::TextureFormat::Rgba8UnormSrgb
            );
        let first = self.render_stats.passes == 0;
        self.render_stats.direct_present = direct && (first || self.render_stats.direct_present);
        let main_format = if dither_output {
            wgpu::TextureFormat::Rgba16Float
        } else {
            working
        };
        let retain = retain_enabled
            && !direct
            && !reads_root
            && RetainedRoot::eligible(&scene, size, main_format);
        let previous = if output.is_some() {
            self.retained.take()
        } else {
            self.retained.clone()
        };
        let retained = previous.filter(|root| {
            retain && root.submitted && root.output == output_format && root.image.size == size
        });
        let damage = retained
            .as_ref()
            .map_or(crate::DamageRect::Full, |root| root.damage(&scene));
        let deferred_root = output.is_none().then(SurfaceGraph::default);
        let mut output = Image::output(output.unwrap_or(&self.white), format, size);
        if deferred_root.is_some() {
            // The white view is a unique target identity while the window view
            // is unavailable. Deferred passes never attach or read that view.
            output.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let mut frame = Frame {
            gpu,
            scene,
            compositor: self,
            command,
            graph: RenderGraph::default(),
            pending: None,
            main_pass: None,
            main_inputs: AttachmentLifetimes::new(),
            deferred_root,
            clears: HashMap::new(),
            opaque: Vec::new(),
            root: output.view.clone(),
            main: output.view.clone(),
            capture,
            frame_images: None,
            size,
            damage_scissor: None,
            baking_subtree: false,
            working,
            straight_alpha,
            finishing_output: false,
            encode_error: None,
        };
        let main = if let Some(retained) = retained {
            retained.image
        } else if retain {
            Image::new(gpu, size, main_format, 1)
        } else if direct {
            output.clone()
        } else {
            frame.take(size, main_format)
        };
        frame.main = main.view.clone();
        let clip = match damage {
            crate::DamageRect::Full => {
                frame.clear(&main.view);
                Some(Clip::default())
            }
            _ => {
                frame.compositor.render_stats.partial_redraw = true;
                damage
                    .pixels(size)
                    .map(|area| frame.clear_damage(&main, area))
                    .transpose()?
            }
        };
        if let Some(clip) = clip
            && !frame.scene.tree.layers.is_empty()
        {
            frame.layers(Some(visible_root), &main, IDENTITY, &clip)?;
        }
        frame.finishing_output = true;
        frame.finish(&main.view);
        let mut retained = None;
        if !direct {
            frame.clear(&output.view);
            frame.full(
                &main,
                None,
                &Coverage::default(),
                &output,
                output.area(),
                if p3 {
                    FullPass::DisplayP3
                } else if dither_output {
                    FullPass::DitherOutput
                } else if format.is_srgb() || linear {
                    FullPass::Copy
                } else {
                    FullPass::EncodeSrgb
                },
            )?;
            if retain {
                retained = Some(RetainedRoot {
                    image: main,
                    output: output_format,
                    revision: frame.scene.revision,
                    submitted: false,
                });
            } else {
                frame.release(main);
            }
        }
        frame.flush_graph();
        if let Some(error) = frame.encode_error {
            return Err(error);
        }
        let cache = frame.compositor.subtrees.stats();
        let damage_pct = frame
            .scene
            .damage
            .fraction(crate::PixelSize::new(size[0], size[1]))
            * 100.0;
        tracing::event!(
            name: "subtree_cache",
            target: "impasto::cache",
            tracing::Level::DEBUG,
            trace_event = true,
            hits = cache.hits,
            misses = cache.misses,
            evictions = cache.evictions,
            cache_bytes = cache.bytes,
            damage_pct
        );
        Ok(PreparedOutput {
            root: frame.deferred_root.unwrap_or_default(),
            tail: frame.command,
            retained,
        })
    }
}

fn frame_encoder(gpu: &Gpu) -> wgpu::CommandEncoder {
    gpu.device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("impasto frame"),
        })
}

fn opaque_tail(scene: &Scene<'_>, size: [u32; 2]) -> (LayerId, u32) {
    let root = scene.tree.root;
    if scene.slice.is_some() {
        return (root, 0);
    }
    let layers = &scene.tree.layers;
    let mut tail = root;
    let mut hidden = 0u32;
    while let Some(next) = layers.get(tail.0 as usize).and_then(|layer| layer.next) {
        tail = next;
        hidden += 1;
    }
    if hidden == 0 {
        return (root, 0);
    }
    let covered = layers.get(tail.0 as usize).is_some_and(|layer| {
        let LayerKind::Picture { picture } = layer.kind else {
            return false;
        };
        let Some(picture) = scene.pictures.get(&picture) else {
            return false;
        };
        if !picture.groups.is_empty() || picture.reads_root || picture.has_shader_effect {
            return false;
        }
        let [width, height] = size.map(|value| value as f32 / scene.scale);
        picture.scopes.iter().any(|scope| {
            let state = &scope.state;
            if state.clip.scissor.is_some()
                || state.clip.hard_scissor.is_some()
                || !state.clip.shapes.is_empty()
                || state.transform != IDENTITY
            {
                return false;
            }
            scope.draws.iter().any(|draw| match draw {
                Draw::Fill(color) => color[3] == 1.0,
                Draw::Rects(run) if run.blend == BlendKey::SrcOver => {
                    run.rects.iter().any(|rect| {
                        let [x, y, w, h] = rect.rect;
                        let x = x + picture.origin.0;
                        let y = y + picture.origin.1;
                        rect.kind == batch::RECT_FILL
                            && rect.color[3] == 1.0
                            && rect.gradient_b[3] == 1.0
                            && rect.flags == 0
                            && rect.radius == 0.0
                            && rect.corners.is_none()
                            && x <= 0.0
                            && y <= 0.0
                            && x + w >= width
                            && y + h >= height
                    })
                }
                _ => false,
            })
        }) && picture
            .scopes
            .iter()
            .flat_map(|scope| &scope.draws)
            .all(Draw::source_over)
    });
    if covered { (tail, hidden) } else { (root, 0) }
}

fn scene_reads_root(scene: &Scene<'_>, mut current: Option<LayerId>) -> bool {
    use engine::scene::BlendMode;
    while let Some(id) = current {
        let Some(layer) = scene.tree.layers.get(id.0 as usize) else {
            return false;
        };
        current = layer.next;
        if scene.slice.is_some_and(|slice| !slice.contains(&id)) {
            continue;
        }
        match layer.kind {
            LayerKind::BackdropBlur { .. }
            | LayerKind::BackdropMaterial { .. }
            | LayerKind::Blend {
                mode: BlendMode::Multiply,
                ..
            } => return true,
            LayerKind::Picture { picture } => {
                if scene
                    .pictures
                    .get(&picture)
                    .is_some_and(|picture| picture.reads_root)
                {
                    return true;
                }
            }
            LayerKind::Clip { .. }
            | LayerKind::CornerClip { .. }
            | LayerKind::Offset { .. }
            | LayerKind::Transform { .. }
                if scene_reads_root(scene, layer.first_child) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

#[derive(Clone, Copy)]
enum ImageSource<'a> {
    External(&'a crate::GpuImage),
    Pooled(&'a Image),
}

impl<'a> ImageSource<'a> {
    fn material(self, frame: &mut Frame<'_>, nearest: bool) -> wgpu::BindGroup {
        let (view, cache) = match self {
            Self::External(image) => (image.view(), image.material.as_ref()),
            Self::Pooled(image) => (&image.view, image.material.as_ref()),
        };
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
        group
    }

    fn nv12(self) -> Option<&'a crate::textures::Nv12Planes> {
        match self {
            Self::External(image) => image.nv12(),
            Self::Pooled(_) => None,
        }
    }

    fn opaque(self) -> bool {
        matches!(self, Self::External(image) if image.opaque)
    }

    fn premultiplied_srgb(self) -> bool {
        matches!(self, Self::External(image) if image.premultiplied_srgb)
    }
}

struct PictureReplay<'a> {
    layer: LayerId,
    picture_id: PictureId,
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
    graph: RenderGraph<PlannedOperation>,
    pending: Option<DrawPass>,
    // Keep the main target open logically while encoding its input textures.
    // A target read or an incompatible attachment ends the current segment.
    main_pass: Option<DrawPass>,
    main_inputs: AttachmentLifetimes<wgpu::TextureView, Image>,
    deferred_root: Option<SurfaceGraph<DeferredRootPass>>,
    clears: HashMap<wgpu::TextureView, wgpu::Color>,
    opaque: Vec<[f32; 4]>,
    root: wgpu::TextureView,
    main: wgpu::TextureView,
    capture: Option<(&'a mut Capture, Option<usize>)>,
    frame_images: Option<image_array::FrameImages<'a>>,
    size: [u32; 2],
    damage_scissor: Option<[u32; 4]>,
    baking_subtree: bool,
    working: wgpu::TextureFormat,
    straight_alpha: bool,
    finishing_output: bool,
    encode_error: Option<Status>,
}

struct DrawPass {
    format: wgpu::TextureFormat,
    target: wgpu::TextureView,
    size: [u32; 2],
    calls: Vec<DrawCall>,
    reads: Vec<wgpu::Texture>,
    spatial: Option<Box<SpatialBatcher<DrawBindings>>>,
    draws: u32,
    shader: Option<engine::scene::ShaderProgramId>,
}

/// Encoded offscreen work with ordered points that still need the drawable.
pub(crate) struct PreparedOutput {
    root: SurfaceGraph<DeferredRootPass>,
    tail: wgpu::CommandEncoder,
    retained: Option<RetainedRoot>,
}

impl PreparedOutput {
    pub fn finish(
        mut self,
        output: &wgpu::TextureView,
        compositor: &mut Compositor,
    ) -> (Vec<wgpu::CommandBuffer>, wgpu::CommandEncoder) {
        let commands = self
            .root
            .encode(&mut self.tail, |pass, command| pass.encode(command, output));
        compositor.retained = self.retained;
        (commands, self.tail)
    }
}

enum PlannedOperation {
    Render(PlannedPass),
    Canvas(Box<CanvasPass>),
    Copy {
        source: wgpu::Texture,
        origin: wgpu::Origin3d,
        target: wgpu::Texture,
        size: wgpu::Extent3d,
    },
    Compute {
        pipeline: wgpu::ComputePipeline,
        material: wgpu::BindGroup,
        offset: u32,
        groups: [u32; 2],
        stamps: Option<PassStamps>,
    },
}

struct PlannedPass {
    target: wgpu::TextureView,
    pass: DeferredRootPass,
    final_output: bool,
}

struct DeferredRootPass {
    segment: RootSegment,
    load: wgpu::LoadOp<wgpu::Color>,
    stamps: Option<PassStamps>,
}

impl DeferredRootPass {
    fn encode(self, command: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
        let stamps = self.stamps.as_ref().map(PassStamps::render);
        self.segment.encode(command, output, self.load, stamps);
    }
}

struct PassStamps {
    queries: wgpu::QuerySet,
    begin: Option<u32>,
    end: Option<u32>,
}

impl From<wgpu::RenderPassTimestampWrites<'_>> for PassStamps {
    fn from(stamps: wgpu::RenderPassTimestampWrites<'_>) -> Self {
        Self {
            queries: stamps.query_set.clone(),
            begin: stamps.beginning_of_pass_write_index,
            end: stamps.end_of_pass_write_index,
        }
    }
}

impl PassStamps {
    fn render(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: self.begin,
            end_of_pass_write_index: self.end,
        }
    }
}

enum RootSegment {
    Ordered(DrawPass),
    EarlyZ(Box<EarlyZBatch>),
    Depth(Box<DepthPass>),
    Composite(CompositeDraw),
    Clear,
}

impl RootSegment {
    fn take_reads(&mut self) -> Vec<wgpu::Texture> {
        match self {
            Self::Ordered(pass) => std::mem::take(&mut pass.reads),
            Self::Composite(pass) => std::mem::take(&mut pass.reads),
            Self::Depth(pass) => std::mem::take(&mut pass.reads),
            _ => Vec::new(),
        }
    }

    fn encode(
        self,
        command: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        stamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        match self {
            Self::EarlyZ(batch) => batch.encode(command, target, load, stamps),
            Self::Depth(batch) => batch.encode(command, target, load, stamps),
            segment => {
                let mut pass = render(command, target, load, stamps);
                match segment {
                    Self::Ordered(batch) => batch.encode(&mut pass, target),
                    Self::Composite(call) => call.encode(&mut pass),
                    Self::Clear | Self::EarlyZ(_) | Self::Depth(_) => {}
                }
            }
        }
    }
}

struct EarlyZBatch {
    depth: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    globals: DynamicGroup,
    vertices: VertexSlice,
    count: u32,
    edges: Option<(wgpu::RenderPipeline, VertexSlice, u32)>,
    mask: wgpu::BindGroup,
}

impl EarlyZBatch {
    fn encode(
        &self,
        command: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        stamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let mut pass = command.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("impasto opaque front-to-back"),
            timestamp_writes: stamps,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals.group, self.globals.offsets());
        pass.set_vertex_buffer(0, self.vertices.slice());
        pass.draw(0..6, 0..self.count);
        if let Some((pipeline, vertices, count)) = &self.edges {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &self.mask, &[]);
            pass.set_vertex_buffer(0, vertices.slice());
            pass.draw(0..6, 0..*count);
        }
    }
}

enum DrawCall {
    Immediate(ImmediateDraw),
    Table(TableDraw),
    Mesh(DirectMesh),
    Composite(CompositeDraw),
    Resident {
        bundle: wgpu::RenderBundle,
        scissor: Option<[u32; 4]>,
    },
}

#[derive(Clone)]
struct DirectMesh {
    pipeline: wgpu::RenderPipeline,
    globals: DynamicGroup,
    material: wgpu::BindGroup,
    geometry: batch::GpuMeshBuffer,
    paint: VertexSlice,
    indices: u32,
    scissor: Option<[u32; 4]>,
}

impl DirectMesh {
    fn encode(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals.group, self.globals.offsets());
        pass.set_bind_group(1, &self.material, &[]);
        pass.set_vertex_buffer(0, self.geometry.vertices.slice());
        pass.set_vertex_buffer(1, self.paint.slice());
        pass.set_index_buffer(self.geometry.indices.slice(), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.indices, 0, 0..1);
    }
}

struct TableDraw {
    bindings: DrawBindings,
    bytes: Vec<u8>,
    instances: u32,
    bounds: Vec<Bounds>,
    kind: PipelineKind,
    blend: BlendKey,
    core: Option<OpaqueCore>,
}

struct OpaqueCore {
    bytes: Vec<u8>,
    bounds: Vec<Bounds>,
    area: f32,
    complete: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum OpaqueClass {
    Ordered,
    Complete,
    Fringe,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct DrawBindings {
    pipeline: wgpu::RenderPipeline,
    globals: DynamicGroup,
    material: wgpu::BindGroup,
    mask: Option<wgpu::BindGroup>,
    vertex_count: u32,
    opaque: OpaqueClass,
}

struct ImmediateDraw {
    bindings: DrawBindings,
    vertices: VertexSlice,
    scissor: Option<[u32; 4]>,
    instances: u32,
}

impl ImmediateDraw {
    fn encode(&self, pass: &mut wgpu::RenderPass<'_>) {
        let bindings = &self.bindings;
        pass.set_pipeline(&bindings.pipeline);
        pass.set_bind_group(0, &bindings.globals.group, bindings.globals.offsets());
        pass.set_bind_group(1, &bindings.material, &[]);
        if let Some(mask) = &bindings.mask {
            pass.set_bind_group(2, mask, &[]);
        }
        pass.set_vertex_buffer(0, self.vertices.slice());
        pass.draw(0..bindings.vertex_count, 0..self.instances);
    }
}

#[derive(Clone)]
struct CompositeDraw {
    kind: PipelineKind,
    blend: BlendKey,
    reads: Vec<wgpu::Texture>,
    pipeline: wgpu::RenderPipeline,
    globals: DynamicGroup,
    source: wgpu::BindGroup,
    composite: Option<wgpu::BindGroup>,
    effect_values: Option<(u32, DynamicGroup)>,
    immediates: Option<ParamsUniform>,
    viewport: [u32; 4],
}

impl CompositeDraw {
    fn encode(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.encode_depth_range(pass, [0.0, 1.0]);
    }

    fn encode_depth_range(&self, pass: &mut wgpu::RenderPass<'_>, [min, max]: [f32; 2]) {
        let [x, y, w, h] = self.viewport.map(|value| value as f32);
        pass.set_viewport(x, y, w, h, min, max);
        pass.set_pipeline(&self.pipeline);
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

impl DrawPass {
    fn read(&mut self, image: &Image) {
        if !self.reads.contains(&image.texture) {
            self.reads.push(image.texture.clone());
        }
    }

    fn barrier(&mut self, next: usize) {
        if let Some(spatial) = &mut self.spatial {
            spatial.barrier(next);
        }
    }

    fn encode(&self, pass: &mut wgpu::RenderPass<'_>, target: &wgpu::TextureView) {
        let mut scissor = None;
        set_scissor(pass, Some([0, 0, self.size[0], self.size[1]]));
        let mut last_pipeline = None;
        let mut last_globals = None;
        let mut last_material = None;
        let mut last_mask = None;
        let mut draws = self.calls.iter().peekable();
        while let Some(draw) = draws.next() {
            let next = match draw {
                DrawCall::Immediate(call) => call.scissor,
                DrawCall::Mesh(call) => call.scissor,
                DrawCall::Composite(_) | DrawCall::Table(_) => None,
                DrawCall::Resident { scissor, .. } => *scissor,
            };
            if next != scissor {
                set_scissor(
                    pass,
                    Some(next.unwrap_or([0, 0, self.size[0], self.size[1]])),
                );
                scissor = next;
            }
            let call = match draw {
                DrawCall::Immediate(call) => call,
                DrawCall::Table(_) => continue,
                DrawCall::Mesh(call) => {
                    call.encode(pass);
                    last_pipeline = None;
                    last_globals = None;
                    last_material = None;
                    last_mask = None;
                    continue;
                }
                DrawCall::Composite(call) => {
                    call.encode(pass);
                    pass.set_viewport(
                        0.0,
                        0.0,
                        target.texture().width() as f32,
                        target.texture().height() as f32,
                        0.0,
                        1.0,
                    );
                    last_pipeline = None;
                    last_globals = None;
                    last_material = None;
                    last_mask = None;
                    continue;
                }

                DrawCall::Resident { bundle, .. } => {
                    let adjacent = std::iter::from_fn(|| match draws.peek()? {
                        DrawCall::Resident { bundle, scissor } if *scissor == next => {
                            draws.next();
                            Some(bundle)
                        }
                        _ => None,
                    });
                    pass.execute_bundles(std::iter::once(bundle).chain(adjacent));
                    last_pipeline = None;
                    last_globals = None;
                    last_material = None;
                    last_mask = None;
                    continue;
                }
            };
            if last_pipeline != Some(&call.bindings.pipeline) {
                pass.set_pipeline(&call.bindings.pipeline);
                last_pipeline = Some(&call.bindings.pipeline);
            }
            let globals = (
                &call.bindings.globals.group,
                call.bindings.globals.offsets(),
            );
            if last_globals != Some(globals) {
                pass.set_bind_group(0, globals.0, globals.1);
                last_globals = Some(globals);
            }
            if last_material != Some(&call.bindings.material) {
                pass.set_bind_group(1, &call.bindings.material, &[]);
                last_material = Some(&call.bindings.material);
            }
            if let Some(mask) = &call.bindings.mask
                && last_mask != Some(mask)
            {
                pass.set_bind_group(2, mask, &[]);
                last_mask = Some(mask);
            }
            pass.set_vertex_buffer(0, call.vertices.slice());
            pass.draw(0..call.bindings.vertex_count, 0..call.instances);
        }
    }

    fn new(target: &Image) -> Self {
        Self {
            format: target.format,
            target: target.view.clone(),
            size: target.logical,
            calls: Vec::new(),
            reads: Vec::new(),
            spatial: None,
            draws: 0,
            shader: None,
        }
    }
}

#[derive(Clone, Copy)]
enum FullPass {
    Copy,
    DitherOutput,
    DisplayP3,
    EncodeSrgb,
    Over {
        blend: BlendKey,
        opacity: f32,
    },
    Multiply {
        opacity: f32,
    },
    Backdrop,
    BackdropMaterial {
        material: engine::scene::BackdropMaterial,
        top: f32,
        inverse_height: f32,
    },
    ColorFilter(engine::scene::ColorMatrix),
    BlurDown {
        radius: f32,
        backdrop: bool,
    },
    BlurUp {
        radius: f32,
        backdrop: bool,
    },
    Gaussian {
        sigma: f32,
        vertical: bool,
        backdrop: bool,
    },
    MaskIntersect,
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

    fn distance_field(self, size: f32) -> bool {
        size * self.pixels > 36.0 || !self.aligned || (self.matrix[0] - 1.0).abs() > 0.001
    }

    fn vector(self, size: f32) -> bool {
        let pixels = size * self.pixels;
        pixels > 96.0 || (pixels > 48.0 && (self.matrix[0] - 1.0).abs() > 0.01)
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
    stencil: Vec<ClipShape>,
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
    #[expect(
        clippy::too_many_arguments,
        reason = "a recorded draw owns its pipeline and resources"
    )]
    fn record<T: bytemuck::Pod>(
        &mut self,
        target: &Image,
        pipeline: wgpu::RenderPipeline,
        globals: DynamicGroup,
        material: wgpu::BindGroup,
        mask: Option<wgpu::BindGroup>,
        vertices: &[T],
        scissor: Option<[u32; 4]>,
        vertex_count: u32,
    ) -> Result<(), Status> {
        let limit = FrameArena::vertex_limit::<T>(self.gpu);
        if limit == 0 {
            return Err(Status::OutOfMemory);
        }
        for chunk in vertices.chunks(limit) {
            self.compositor.render_stats.draws += 1;
            let call = DrawCall::Immediate(ImmediateDraw {
                bindings: DrawBindings {
                    pipeline: pipeline.clone(),
                    globals: globals.clone(),
                    material: material.clone(),
                    mask: mask.clone(),
                    vertex_count,
                    opaque: OpaqueClass::Ordered,
                },
                vertices: self.compositor.arena.vertices(self.gpu, chunk)?,
                scissor: scissor.map(|area| target.local(area)),
                instances: draw_count(chunk.len())?,
            });
            let pass = self.pending_pass(target);
            pass.barrier(pass.calls.len() + 1);
            pass.calls.push(call);
            pass.draws += 1;
        }
        Ok(())
    }

    fn read_mask(&mut self, target: &Image, clip: &Coverage) {
        if let Some(mask) = &clip.mask {
            self.pending_pass(target).read(mask);
        }
    }

    fn pending_pass(&mut self, target: &Image) -> &mut DrawPass {
        if target.view == self.main {
            return self.main_pass.get_or_insert_with(|| DrawPass::new(target));
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pass| pass.target != target.view)
        {
            self.flush_draws();
        }
        self.pending.get_or_insert_with(|| DrawPass::new(target))
    }

    fn flush_draws(&mut self) {
        if let Some(batch) = self.pending.take() {
            self.encode_draws(batch);
        }
    }

    fn flush_target(&mut self, view: &wgpu::TextureView) {
        self.flush_draws();
        if *view == self.main {
            self.flush_main();
        }
    }

    fn flush_main(&mut self) {
        if let Some(batch) = self.main_pass.take() {
            self.encode_draws(batch);
        }
        for image in self.main_inputs.finish() {
            self.compositor.offscreen_live_bytes -= image.bytes();
            self.compositor
                .pool
                .recycle(image, self.compositor.render_graph);
        }
    }

    fn encode_draws(&mut self, mut batch: DrawPass) {
        match self.prepare_depth(&batch) {
            Ok(Some((depth, draws))) => {
                let attachment = depth.depth.clone();
                self.encode_pass(
                    &batch.target,
                    RootSegment::Depth(Box::new(depth)),
                    GpuPassKind::Main,
                    draws,
                    batch.shader,
                );
                self.release(attachment);
                return;
            }
            Err(error) => {
                self.encode_error = Some(error);
                return;
            }
            Ok(None) => {}
        }
        for call in &mut batch.calls {
            if let DrawCall::Table(table) = call {
                match self.compositor.arena.vertices(self.gpu, &table.bytes) {
                    Ok(vertices) => {
                        *call = DrawCall::Immediate(ImmediateDraw {
                            bindings: table.bindings.clone(),
                            vertices,
                            scissor: None,
                            instances: table.instances,
                        });
                    }
                    Err(error) => {
                        self.encode_error = Some(error);
                        return;
                    }
                }
            }
        }
        let kind = self.pass_kind(&batch.target, batch.format);
        let target = batch.target.clone();
        let draws = batch.draws;
        let shader = batch.shader;
        self.encode_pass(&target, RootSegment::Ordered(batch), kind, draws, shader);
    }

    fn encode_pass(
        &mut self,
        target: &wgpu::TextureView,
        segment: RootSegment,
        kind: GpuPassKind,
        draws: u32,
        shader: Option<engine::scene::ShaderProgramId>,
    ) {
        self.count_pass(target);
        let load = self.load(target);
        let stamps = self
            .capture
            .as_mut()
            .and_then(|(capture, slice)| {
                capture.tagged_pass(
                    if shader.is_some() {
                        GpuPassKind::ShaderEffect
                    } else {
                        kind
                    },
                    *slice,
                    draws,
                    shader.map(GpuPassSource::ShaderEffect),
                )
            })
            .map(|stamps| PassStamps {
                queries: stamps.query_set.clone(),
                begin: stamps.beginning_of_pass_write_index,
                end: stamps.end_of_pass_write_index,
            });
        let pass = DeferredRootPass {
            segment,
            load,
            stamps,
        };
        let mut node = PlannedPass {
            target: target.clone(),
            pass,
            final_output: self.finishing_output,
        };
        if self.compositor.render_graph {
            let inputs = node.pass.segment.take_reads();
            self.graph.record(
                PlannedOperation::Render(node),
                *target == self.main || *target == self.root,
                inputs,
                [target.texture().clone()],
            );
        } else {
            self.encode_planned(node);
        }
    }

    fn flush_graph(&mut self) {
        let passes = self.graph.schedule();
        for pass in passes {
            self.encode_operation(pass);
        }
    }

    fn encode_operation(&mut self, op: PlannedOperation) {
        match op {
            PlannedOperation::Render(pass) => self.encode_planned(pass),
            PlannedOperation::Canvas(pass) => pass.encode(&mut self.command),
            PlannedOperation::Copy {
                source,
                origin,
                target,
                size,
            } => {
                self.command.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        origin,
                        ..source.as_image_copy()
                    },
                    target.as_image_copy(),
                    size,
                );
            }
            PlannedOperation::Compute {
                pipeline,
                material,
                offset,
                groups,
                stamps,
            } => {
                let timestamp_writes =
                    stamps
                        .as_ref()
                        .map(|stamps| wgpu::ComputePassTimestampWrites {
                            query_set: &stamps.queries,
                            beginning_of_pass_write_index: stamps.begin,
                            end_of_pass_write_index: stamps.end,
                        });
                let mut pass = self
                    .command
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("impasto compute blur"),
                        timestamp_writes,
                    });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &material, &[offset]);
                pass.dispatch_workgroups(groups[0], groups[1], 1);
            }
        }
    }

    fn encode_planned(&mut self, node: PlannedPass) {
        if let Some(root) = &mut self.deferred_root
            && node.target == self.root
        {
            if node.final_output {
                root.finish(node.pass);
            } else {
                let prefix = std::mem::replace(&mut self.command, frame_encoder(self.gpu));
                root.segment(prefix, node.pass);
            }
        } else {
            node.pass.encode(&mut self.command, &node.target);
        }
    }

    fn pass_kind(&self, view: &wgpu::TextureView, format: wgpu::TextureFormat) -> GpuPassKind {
        if *view == self.main {
            GpuPassKind::Main
        } else if format == wgpu::TextureFormat::R8Unorm {
            GpuPassKind::ClipMask
        } else {
            GpuPassKind::OffscreenEffect
        }
    }

    fn count_pass(&mut self, target: &wgpu::TextureView) {
        self.compositor.render_stats.passes += 1;
        self.compositor.render_stats.offscreen_passes += u32::from(target != &self.root);
    }

    fn load(&mut self, target: &wgpu::TextureView) -> wgpu::LoadOp<wgpu::Color> {
        self.clears
            .remove(target)
            .map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear)
    }

    fn clear(&mut self, view: &wgpu::TextureView) {
        self.clears.insert(view.clone(), wgpu::Color::TRANSPARENT);
    }

    fn finish(&mut self, view: &wgpu::TextureView) {
        self.flush_target(view);
        if self.clears.contains_key(view) {
            let kind = self.pass_kind(view, wgpu::TextureFormat::Rgba8UnormSrgb);
            self.encode_pass(view, RootSegment::Clear, kind, 0, None);
        }
    }

    fn take(&mut self, size: [u32; 2], format: wgpu::TextureFormat) -> Image {
        self.take_area([0, 0, size[0], size[1]], format, 1)
    }

    fn take_area(&mut self, area: [u32; 4], format: wgpu::TextureFormat, samples: u32) -> Image {
        self.take_area_with_usage(area, format, samples, Image::usage(samples))
    }

    fn take_area_with_usage(
        &mut self,
        area: [u32; 4],
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Image {
        let [_, _, w, h] = area;
        let size = [w, h].map(|side| side.next_multiple_of(64).min(self.gpu.caps.max_texture_2d));
        self.take_image(area, size, format, samples, usage)
    }

    fn take_image(
        &mut self,
        area: [u32; 4],
        size: [u32; 2],
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Image {
        let [x, y, w, h] = area;
        let compositor = &mut self.compositor;
        // Reusing the most recently released attachment keeps material roles
        // stable in a repeated scene and avoids rotating bind-group keys.
        let mut image = compositor
            .pool
            .take(size, format, samples, usage)
            .unwrap_or_else(|| {
                let image = Image::with_usage(self.gpu, size, format, samples, usage);
                compositor.render_stats.offscreen_alloc_bytes += image.bytes();
                image
            });
        compositor.offscreen_live_bytes += image.bytes();
        compositor.render_stats.offscreen_peak_bytes = compositor
            .render_stats
            .offscreen_peak_bytes
            .max(compositor.offscreen_live_bytes);
        image.logical = [w, h];
        image.origin = [x, y];
        image
    }

    fn take_attachment(
        &mut self,
        area: [u32; 4],
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Image {
        use wgpu::TextureUsages as U;
        let mut usage = U::RENDER_ATTACHMENT;
        if self.compositor.transient {
            usage |= U::TRANSIENT_ATTACHMENT;
        }
        let image = self.take_area_with_usage(area, format, samples, usage);
        if self.compositor.transient {
            self.compositor.render_stats.transient_bytes += image.bytes();
        }
        image
    }

    fn take_exact_attachment(&mut self, area: [u32; 4], format: wgpu::TextureFormat) -> Image {
        use wgpu::TextureUsages as U;
        let mut usage = U::RENDER_ATTACHMENT;
        if self.compositor.transient {
            usage |= U::TRANSIENT_ATTACHMENT;
        }
        let image = self.take_image(area, [area[2], area[3]], format, 1, usage);
        if self.compositor.transient {
            self.compositor.render_stats.transient_bytes += image.bytes();
        }
        image
    }

    fn release(&mut self, image: Image) {
        if self.main_inputs.contains(&image.view) {
            let bytes = image.bytes();
            self.main_inputs.retire(image, bytes);
            let max_inputs = if self.compositor.render_graph {
                usize::MAX
            } else {
                POOL_LIMIT
            };
            if self.main_inputs.exceeds(max_inputs, POOL_BYTES) {
                self.flush_target(&self.main.clone());
            }
            return;
        }
        self.compositor.offscreen_live_bytes -= image.bytes();
        self.compositor
            .pool
            .recycle(image, self.compositor.render_graph);
    }

    fn layer_bounds(
        &self,
        mut current: Option<LayerId>,
        transform: Affine,
    ) -> Result<PictureBounds, Status> {
        let mut bounds = PictureBounds::Empty;
        while let Some(id) = current {
            let layer = self
                .scene
                .tree
                .layers
                .get(id.0 as usize)
                .ok_or(Status::BadScene)?;
            current = layer.next;
            if self.scene.slice.is_some_and(|slice| !slice.contains(&id)) {
                continue;
            }
            bounds = bounds.union(self.layer_bound(layer, transform)?);
        }
        Ok(bounds)
    }

    fn layer_bound(
        &self,
        layer: &engine::scene::Layer,
        transform: Affine,
    ) -> Result<PictureBounds, Status> {
        let children = layer.first_child;
        Ok(match layer.kind {
            LayerKind::PlatformView { .. } => PictureBounds::Empty,
            LayerKind::Fill { bounds, .. } => PictureBounds::rect(bounds).transform(transform),
            LayerKind::Picture { picture } => {
                let picture = self.scene.pictures.get(&picture).ok_or(Status::BadScene)?;
                let origin =
                    compiler::compose(transform, translation(picture.origin.0, picture.origin.1));
                picture.bounds_in(origin, self.scene.scale).0
            }
            LayerKind::Offset { .. } | LayerKind::Transform { .. } => self.layer_bounds(
                children,
                compiler::compose(transform, compiler::layer_transform(&layer.kind)),
            )?,
            LayerKind::CornerClip {
                bounds: [x, y, w, h],
                ..
            } => {
                let child = self.layer_bounds(children, transform)?;
                if let PictureBounds::Rect(clip) =
                    PictureBounds::rect([x, y, w, h]).transform(transform)
                {
                    child.clip(clip)
                } else {
                    child
                }
            }
            LayerKind::Clip { x, y, w, h, .. } => {
                let child = self.layer_bounds(children, transform)?;
                if let PictureBounds::Rect(clip) =
                    PictureBounds::rect([x, y, w, h]).transform(transform)
                {
                    child.clip(clip)
                } else {
                    child
                }
            }
            LayerKind::Blur { sigma, .. } => self
                .layer_bounds(children, transform)?
                .expand(3.0 * sigma.max(0.0) * compiler::effect_scale(transform)),
            LayerKind::BackdropBlur { x, y, w, h, .. } => self
                .layer_bounds(children, transform)?
                .union(PictureBounds::rect([x, y, w, h]).transform(transform)),
            LayerKind::BackdropMaterial { bounds, .. } => self
                .layer_bounds(children, transform)?
                .union(PictureBounds::rect(bounds).transform(transform)),
            _ => self.layer_bounds(children, transform)?,
        })
    }

    fn source_over_layers(&self, mut current: Option<LayerId>) -> bool {
        while let Some(id) = current {
            let Some(layer) = self.scene.tree.layers.get(id.0 as usize) else {
                return false;
            };
            current = layer.next;
            match layer.kind {
                LayerKind::Picture { picture } => {
                    if !self.scene.pictures.get(&picture).is_some_and(|picture| {
                        picture.groups.is_empty()
                            && picture
                                .scopes
                                .iter()
                                .flat_map(|scope| &scope.draws)
                                .all(Draw::source_over)
                    }) {
                        return false;
                    }
                }
                LayerKind::Clip { .. }
                | LayerKind::CornerClip { .. }
                | LayerKind::Offset { .. }
                | LayerKind::Transform { .. } => {
                    if !self.source_over_layers(layer.first_child) {
                        return false;
                    }
                }
                LayerKind::PlatformView { .. } | LayerKind::Fill { .. } => {}
                _ => return false,
            }
        }
        true
    }

    fn layers(
        &mut self,
        mut current: Option<LayerId>,
        target: &Image,
        transform: Affine,
        clip: &Clip,
    ) -> Result<(), Status> {
        let hidden = self.occluded_siblings(current, transform, clip, target)?;
        while let Some(id) = current {
            let layer = self
                .scene
                .tree
                .layers
                .get(id.0 as usize)
                .ok_or(Status::BadScene)?;
            current = layer.next;
            if hidden.contains(&id) || self.scene.slice.is_some_and(|slice| !slice.contains(&id)) {
                continue;
            }
            let children = layer.first_child;
            match layer.kind {
                LayerKind::PlatformView { .. } => {}
                LayerKind::Fill {
                    bounds,
                    radius,
                    corners,
                    color,
                } => {
                    if color[3] > 0.0 && bounds[2] > 0.0 && bounds[3] > 0.0 {
                        let coverage = self.coverage(target, clip)?;
                        self.rects(
                            target,
                            transform,
                            &coverage,
                            &[RectInstance {
                                rect: bounds,
                                radius,
                                corners,
                                color,
                                stroke_width: 0.0,
                                kind: batch::RECT_FILL,
                                flags: 0,
                                stroke_color: [0.0; 4],
                                gradient_a: [0.0; 4],
                                gradient_b: [0.0, 0.0, 0.0, 1.0],
                                shadow: [0.0; 4],
                            }],
                            &[],
                            None,
                        )?;
                    }
                }
                LayerKind::Picture {
                    picture: picture_id,
                } => {
                    let picture = Arc::clone(
                        self.scene
                            .pictures
                            .get(&picture_id)
                            .ok_or(Status::BadScene)?,
                    );
                    let local = compiler::compose(
                        transform,
                        translation(picture.origin.0, picture.origin.1),
                    );
                    let replay = PictureReplay {
                        layer: id,
                        picture_id,
                        picture: &picture,
                        origin: local,
                        group_bounds: if picture.groups.is_empty() {
                            None
                        } else {
                            picture.bounds_in(local, self.scene.scale).1
                        },
                    };
                    if picture.cache_expensive {
                        let candidate = CacheCandidate {
                            slot: CacheSlot {
                                layer: id,
                                group: None,
                            },
                            picture_id,
                            picture: &picture,
                            origin: local,
                            bounds: picture.bounds_in(local, self.scene.scale).0,
                            blur: None,
                        };
                        if self.try_subtree_cache(candidate, target, clip, |frame, target| {
                            frame.replay_picture(&replay, target, clip)
                        })? {
                            continue;
                        }
                    }
                    self.replay_picture(&replay, target, clip)?;
                }
                LayerKind::Offset { .. } | LayerKind::Transform { .. } => {
                    let local = compiler::layer_transform(&layer.kind);
                    let placed = compiler::compose(transform, local);
                    if matches!(layer.kind, LayerKind::Transform { .. })
                        && self.try_transition_snapshot(id, children, target, placed, clip, 1.0)?
                    {
                        continue;
                    }
                    self.layers(children, target, placed, clip)?;
                }
                LayerKind::CornerClip { bounds, corners } => {
                    let mut next = clip.clone();
                    add_corner_clip(&mut next, bounds, 0.0, Some(corners), transform);
                    self.layers(layer.first_child, target, transform, &next)?;
                }
                LayerKind::Clip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius,
                } => {
                    let mut nested = clip.clone();
                    add_rect_clip(&mut nested, [x, y, w, h], corner_radius, transform);
                    self.layers(children, target, transform, &nested)?;
                }
                LayerKind::Opacity { alpha: 0, .. } => {}
                LayerKind::Opacity { alpha, .. } => {
                    if alpha == 255 && self.source_over_layers(children) {
                        self.layers(children, target, transform, clip)?;
                        continue;
                    }
                    if let Some(child_id) = children
                        && let Some(child) = self.scene.tree.layers.get(child_id.0 as usize)
                        && child.next.is_none()
                        && matches!(child.kind, LayerKind::Transform { .. })
                        && self
                            .scene
                            .tree
                            .tracks
                            .iter()
                            .any(|track| track.layer == child_id && track.freeze_subtree)
                    {
                        let placed =
                            compiler::compose(transform, compiler::layer_transform(&child.kind));
                        if self.try_transition_snapshot(
                            child_id,
                            child.first_child,
                            target,
                            placed,
                            clip,
                            f32::from(alpha) / 255.0,
                        )? {
                            continue;
                        }
                    }
                    let group = Group {
                        effect: Effect::Opacity,
                        opacity: f32::from(alpha) / 255.0,
                        sigma: 0.0,
                        bounds: None,
                        content: self.layer_bounds(children, transform)?,
                        transform,
                    };
                    if let Some(id) = children
                        && self.scene.slice.is_none_or(|slice| slice.contains(&id))
                        && let Some(child) = self.scene.tree.layers.get(id.0 as usize)
                        && child.next.is_none()
                        && let LayerKind::Picture { picture } = child.kind
                    {
                        let picture =
                            Arc::clone(self.scene.pictures.get(&picture).ok_or(Status::BadScene)?);
                        let origin = compiler::compose(
                            transform,
                            translation(picture.origin.0, picture.origin.1),
                        );
                        if self.fold_opacity(group, &picture.scopes, 0, target, origin, clip)? {
                            continue;
                        }
                    }
                    if self.try_transition_snapshot(
                        id,
                        children,
                        target,
                        transform,
                        clip,
                        group.opacity,
                    )? {
                        continue;
                    }
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
                LayerKind::Blur { sigma, .. } if sigma <= 0.0 => {
                    self.layers(children, target, transform, clip)?;
                }
                LayerKind::Blur { sigma, .. } => {
                    let group = Group {
                        effect: Effect::Blur,
                        opacity: 1.0,
                        sigma,
                        bounds: None,
                        content: self.layer_bounds(children, transform)?,
                        transform,
                    };
                    if let Some(child) = children
                        .and_then(|id| self.scene.tree.layers.get(id.0 as usize))
                        .filter(|child| child.next.is_none())
                        && let LayerKind::Picture {
                            picture: picture_id,
                        } = child.kind
                        && let Some(picture) = self.scene.pictures.get(&picture_id).cloned()
                    {
                        let origin = compiler::compose(
                            transform,
                            translation(picture.origin.0, picture.origin.1),
                        );
                        let candidate = CacheCandidate {
                            slot: CacheSlot {
                                layer: id,
                                group: None,
                            },
                            picture_id,
                            picture: &picture,
                            origin,
                            bounds: group
                                .content
                                .expand(3.0 * sigma.max(0.0) * compiler::effect_scale(transform)),
                            blur: Some(sigma),
                        };
                        if self.try_subtree_cache(candidate, target, clip, |frame, target| {
                            frame.group(target, transform, clip, group, |frame, target| {
                                frame.layers(children, target, transform, clip)
                            })
                        })? {
                            continue;
                        }
                    }
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
                LayerKind::ColorFilter { ref matrix } => {
                    let group = Group {
                        effect: Effect::ColorFilter(**matrix),
                        opacity: 1.0,
                        sigma: 0.0,
                        bounds: None,
                        content: self.layer_bounds(children, transform)?,
                        transform,
                    };
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
                LayerKind::BackdropBlur { sigma, .. } if sigma <= 0.0 => {
                    self.layers(children, target, transform, clip)?;
                }
                LayerKind::BackdropBlur { x, y, w, h, sigma } => {
                    let group = Group {
                        effect: Effect::BackdropBlur,
                        opacity: 1.0,
                        sigma,
                        bounds: Some([x, y, w, h]),
                        content: PictureBounds::Empty,
                        transform,
                    };
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
                LayerKind::BackdropMaterial {
                    bounds,
                    corners,
                    ref material,
                } => {
                    let group = Group {
                        effect: Effect::BackdropMaterial(**material, corners),
                        opacity: 1.0,
                        sigma: material.sigma,
                        bounds: Some(bounds),
                        content: PictureBounds::Empty,
                        transform,
                    };
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
                LayerKind::Blend { mode, .. } => {
                    let group = Group {
                        effect: Effect::Blend(mode),
                        opacity: 1.0,
                        sigma: 0.0,
                        bounds: None,
                        content: self.layer_bounds(children, transform)?,
                        transform,
                    };
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
                }
            }
        }
        Ok(())
    }

    fn occluded_siblings(
        &mut self,
        first: Option<LayerId>,
        transform: Affine,
        clip: &Clip,
        target: &Image,
    ) -> Result<Vec<LayerId>, Status> {
        const MAX_SIBLINGS: usize = 32;
        let mut siblings = [LayerId(0); MAX_SIBLINGS];
        let mut count = 0;
        let mut current = first;
        if self.scene.slice.is_some() || !clip.shapes.is_empty() {
            return Ok(Vec::new());
        }
        while let Some(id) = current {
            let Some(slot) = siblings.get_mut(count) else {
                return Ok(Vec::new());
            };
            *slot = id;
            count += 1;
            current = self.scene.tree.layers[id.0 as usize].next;
        }
        let Some((&tail, preceding)) = siblings[..count].split_last() else {
            return Ok(Vec::new());
        };
        if preceding.is_empty() {
            return Ok(Vec::new());
        }
        let scale = self.scene.scale;
        let target = target.area().map(|value| value as f32 / scale);
        let Some(cover) = self.opaque_layer_rect(tail, transform, target) else {
            return Ok(Vec::new());
        };
        // A visible backdrop or shader can sample pixels beyond its own bounds.
        if scene_reads_root(&self.scene, first) {
            return Ok(Vec::new());
        }
        let mut hidden = Vec::new();
        for &id in preceding {
            let layer = &self.scene.tree.layers[id.0 as usize];
            if matches!(layer.kind, LayerKind::PlatformView { .. }) {
                continue;
            }
            let bounds = self.layer_bound(layer, transform)?.expand(1.0 / scale);
            if let PictureBounds::Rect(bounds) = bounds
                && intersect(cover, bounds) == bounds
            {
                hidden.push(id);
            }
        }
        self.compositor.render_stats.occluded_siblings += u32::try_from(hidden.len()).unwrap_or(0);
        Ok(hidden)
    }

    fn opaque_layer_rect(
        &self,
        mut id: LayerId,
        mut transform: Affine,
        target: [f32; 4],
    ) -> Option<[f32; 4]> {
        let scale = self.scene.scale;
        let mut limit = target;
        loop {
            let layer = self.scene.tree.layers.get(id.0 as usize)?;
            match layer.kind {
                LayerKind::Offset { .. } | LayerKind::Transform { .. } => {
                    transform =
                        compiler::compose(transform, compiler::layer_transform(&layer.kind));
                }
                LayerKind::Clip {
                    x,
                    y,
                    w,
                    h,
                    corner_radius: 0.0,
                } if transform[1] == 0.0 && transform[2] == 0.0 => {
                    let PictureBounds::Rect(bounds) =
                        PictureBounds::rect([x, y, w, h]).transform(transform)
                    else {
                        return None;
                    };
                    limit = intersect(limit, bounds);
                }
                LayerKind::Picture { picture } => {
                    let picture = self.scene.pictures.get(&picture)?;
                    if picture.reads_root
                        || picture.has_shader_effect
                        || !picture.groups.is_empty()
                        || !picture
                            .scopes
                            .iter()
                            .flat_map(|scope| &scope.draws)
                            .all(Draw::source_over)
                    {
                        return None;
                    }
                    let origin = compiler::compose(
                        transform,
                        translation(picture.origin.0, picture.origin.1),
                    );
                    return picture.scopes.iter().rev().find_map(|scope| {
                        if !scope.state.clip.shapes.is_empty() {
                            return None;
                        }
                        let transform = compiler::compose(origin, scope.state.transform);
                        let scope_clip = merge_clip(&Clip::default(), &scope.state.clip, origin);
                        if !scope_clip.shapes.is_empty() {
                            return None;
                        }
                        let limit = scope_clip
                            .scissor
                            .map_or(limit, |clip| intersect(limit, clip));
                        scope.draws.iter().rev().find_map(|draw| match draw {
                            Draw::Fill(color) if color[3] == 1.0 => Some(limit),
                            Draw::Rects(run)
                                if transform[1] == 0.0
                                    && transform[2] == 0.0
                                    && transform[0].abs() == transform[3].abs() =>
                            {
                                run.rects.iter().rev().find_map(|rect| {
                                    if rect.kind != batch::RECT_FILL
                                        || rect.color[3] != 1.0
                                        || rect.gradient_b[3] != 1.0
                                        || rect.flags != 0
                                        || rect.corners.is_some()
                                    {
                                        return None;
                                    }
                                    let [x, y, w, h] = rect.rect;
                                    let radius = rect.radius.max(0.0).min(w.min(h) * 0.5);
                                    let inset = radius * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
                                    let interior = PictureBounds::rect([
                                        x + inset,
                                        y + inset,
                                        w - 2.0 * inset,
                                        h - 2.0 * inset,
                                    ])
                                    .transform(transform)
                                    .expand(-1.0 / scale);
                                    let PictureBounds::Rect(interior) = interior else {
                                        return None;
                                    };
                                    Some(intersect(limit, interior))
                                })
                            }
                            _ => None,
                        })
                    });
                }
                _ => return None,
            }
            id = layer.first_child?;
            if self.scene.tree.layers.get(id.0 as usize)?.next.is_some() {
                return None;
            }
        }
    }

    fn replay_picture(
        &mut self,
        replay: &PictureReplay<'_>,
        target: &Image,
        clip: &Clip,
    ) -> Result<(), Status> {
        let picture = replay.picture;
        let origin = replay.origin;
        if picture.scope_blocks.is_empty()
            || origin[..4] != IDENTITY[..4]
            || self.scene.scale < picture.scale
        {
            return self.scopes(replay, &picture.scopes, 0, target, clip, 0);
        }
        let block_size = compiler::SCOPES_PER_BLOCK;
        for (block, (&bounds, scopes)) in picture
            .scope_blocks
            .iter()
            .zip(picture.scopes.chunks(block_size))
            .enumerate()
        {
            let bounds = bounds.transform(origin).expand(1.0 / self.scene.scale);
            if self.area(bounds, target).is_some() {
                self.scopes(replay, scopes, 0, target, clip, block * block_size)?;
            }
        }
        Ok(())
    }

    fn scopes(
        &mut self,
        replay: &PictureReplay<'_>,
        mut scopes: &[Scope],
        depth: usize,
        target: &Image,
        parent_clip: &Clip,
        first_scope: usize,
    ) -> Result<(), Status> {
        let origin = replay.origin;
        let scope_end = first_scope + scopes.len();
        while let Some((scope, rest)) = scopes.split_first() {
            let scope_index = scope_end - scopes.len();
            let state = &scope.state;
            if let Some(id) = state.groups.get(depth) {
                let length = scopes
                    .iter()
                    .take_while(|scope| scope.state.groups.get(depth) == Some(id))
                    .count();
                let (group_scopes, remaining) = scopes.split_at(length);
                let group = replay.group(*id)?;
                let transform = compiler::compose(origin, group.transform);
                let clip = merge_clip(parent_clip, &state.clip, origin);
                if group.effect == Effect::Opacity
                    && group.opacity == 1.0
                    && group_scopes
                        .iter()
                        .filter(|scope| !scope.draws.is_empty())
                        .all(|scope| {
                            scope.state.groups.len() == depth + 1
                                && scope.draws.iter().all(Draw::source_over)
                        })
                {
                    self.scopes(
                        replay,
                        group_scopes,
                        depth + 1,
                        target,
                        parent_clip,
                        scope_index,
                    )?;
                    scopes = remaining;
                    continue;
                }
                if !self.fold_opacity(
                    group,
                    group_scopes,
                    depth + 1,
                    target,
                    origin,
                    parent_clip,
                )? {
                    let render = |frame: &mut Self, target: &Image| {
                        frame.group(target, transform, &clip, group, |frame, target| {
                            frame.scopes(
                                replay,
                                group_scopes,
                                depth + 1,
                                target,
                                parent_clip,
                                scope_index,
                            )
                        })
                    };
                    let sigma = group.sigma * compiler::effect_scale(transform);
                    // Retain a soft group inside a mixed picture independently
                    // so its sharp siblings can still follow the live transform.
                    if depth == 0
                        && group.effect == Effect::Blur
                        && group.bounds.is_none()
                        && sigma.is_finite()
                        && sigma * self.scene.scale >= MIN_RESAMPLED_BLUR
                        && self.try_subtree_cache(
                            CacheCandidate {
                                slot: CacheSlot {
                                    layer: replay.layer,
                                    group: Some(*id),
                                },
                                picture_id: replay.picture_id,
                                picture: replay.picture,
                                origin,
                                bounds: group.content.expand(3.0 * sigma),
                                blur: Some(sigma),
                            },
                            target,
                            &clip,
                            render,
                        )?
                    {
                        scopes = remaining;
                        continue;
                    }
                    render(self, target)?;
                }
                scopes = remaining;
                continue;
            }
            scopes = rest;
            if scope.draws.is_empty()
                || self
                    .area(
                        scope
                            .bounds_in(origin, self.scene.scale)
                            .expand(1.0 / self.scene.scale),
                        target,
                    )
                    .is_none()
            {
                continue;
            }
            let transform = compiler::compose(origin, state.transform);
            let clip = merge_clip(parent_clip, &state.clip, origin);
            let stencil = self.compositor.canvas_stencil
                && scope.draws.iter().all(|draw| {
                    matches!(draw, Draw::Mesh(mesh)
                    if mesh.composite == engine::scene::CanvasComposite::SourceOver)
                });
            let coverage = if stencil {
                match self.stencil_coverage(target, &clip) {
                    Some(coverage) => coverage,
                    None => self.coverage(target, &clip)?,
                }
            } else {
                self.coverage(target, &clip)?
            };
            if !coverage.empty {
                if depth == 0
                    && replay.picture.has_resident_candidate
                    && !scope.resident_candidates.is_empty()
                    && self.compositor.pipelines.resident_pictures
                    && target.view == self.main
                    && coverage.mask.is_none()
                    && !self.compositor.early_z
                    && !self.gpu.gradient_lut
                {
                    let mut first = 0;
                    while let Some(draw) = scope.draws.get(first) {
                        let kind = std::mem::discriminant(draw);
                        let count = scope.draws[first..]
                            .iter()
                            .take_while(|next| std::mem::discriminant(*next) == kind)
                            .count();
                        let draws = &scope.draws[first..first + count];
                        if count != 1
                            || !scope.resident_candidates[first]
                            || !self.try_resident_scope(
                                target,
                                transform,
                                &coverage,
                                draws,
                                replay,
                                (scope_index, first),
                            )?
                        {
                            self.draws(replay.layer, target, transform, &coverage, draws)?;
                        }
                        first += count;
                    }
                } else {
                    self.draws(replay.layer, target, transform, &coverage, &scope.draws)?;
                }
            }
        }
        Ok(())
    }

    fn stencil_coverage(&self, target: &Image, clip: &Clip) -> Option<Coverage> {
        let format =
            pipelines::CANVAS_STENCIL.guaranteed_format_features(self.gpu.device.features());
        if clip.shapes.is_empty()
            || clip.shapes.len() > 255
            || !format
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || (self.gpu.caps.msaa4_srgb
                && !format
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4))
        {
            return None;
        }
        let working = self
            .working
            .guaranteed_format_features(self.gpu.device.features());
        let antialiased = working.flags.contains(
            wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        );
        if !clip.shapes.iter().all(|shape| {
            shape.mesh.is_some()
                && shape.path.as_ref().is_some_and(|path| {
                    antialiased || axis_aligned_path(path, shape.transform, self.scene.scale)
                })
        }) {
            return None;
        }
        let target_area = self
            .damage_scissor
            .filter(|_| target.view == self.main)
            .unwrap_or_else(|| target.area());
        let area = match clip.scissor {
            Some(bounds) => scissor(bounds, self.scene.scale, self.size)
                .and_then(|bounds| intersect_pixels(bounds, target_area)),
            None => Some(target_area),
        };
        let Some(area) = area else {
            return Some(Coverage {
                empty: true,
                ..Coverage::default()
            });
        };
        Some(Coverage {
            scissor: (area != target.area()).then_some(area),
            stencil: clip.shapes.clone(),
            ..Coverage::default()
        })
    }

    fn coverage(&mut self, target: &Image, clip: &Clip) -> Result<Coverage, Status> {
        let mut coverage = Coverage::default();
        let mut area = self
            .damage_scissor
            .filter(|_| target.view == self.main)
            .unwrap_or_else(|| target.area());
        if let Some(bounds) = clip.scissor {
            let Some(clipped) = scissor(bounds, self.scene.scale, self.size)
                .and_then(|bounds| intersect_pixels(area, bounds))
            else {
                coverage.empty = true;
                return Ok(coverage);
            };
            area = clipped;
        }
        let mut complex = Vec::new();
        for shape in &clip.shapes {
            let shape_bounds = PictureBounds::rect(shape.rect).transform(shape.transform);
            let Some(clipped) = self
                .area(shape_bounds, target)
                .and_then(|bounds| intersect_pixels(area, bounds))
            else {
                coverage.empty = true;
                return Ok(coverage);
            };
            area = clipped;
            let [a, b, c, d, ..] = shape.transform;
            if shape.mesh.is_none()
                && b.abs() < 0.0001
                && c.abs() < 0.0001
                && a > 0.0
                && (a - d).abs() < 0.0001
                && coverage.count < 4
            {
                let [x, y, w, h] = shape.rect;
                let (cx, cy) = compiler::apply(shape.transform, x + w * 0.5, y + h * 0.5);
                let scale = self.scene.scale;
                coverage.boxes[coverage.count] = [
                    cx * scale,
                    cy * scale,
                    w * a * scale * 0.5,
                    h * a * scale * 0.5,
                ];
                if let Some(corners) = shape.corners {
                    coverage.radii[coverage.count] = corners.radii.map(|radius| radius * a * scale);
                    coverage.curves[coverage.count] = corners.curve;
                } else {
                    coverage.radii[coverage.count] = [shape.radius * a * scale; 4];
                }
                coverage.count += 1;
            } else {
                complex.push(shape);
            }
        }
        coverage.scissor = (area != target.area()).then_some(area);
        if complex.is_empty() {
            return Ok(coverage);
        }
        let key = MaskKey {
            area,
            scale: self.scene.scale.to_bits(),
            shapes: complex
                .iter()
                .map(|shape| MaskShapeKey {
                    rect: shape.rect.map(f32::to_bits),
                    radius: shape.radius.to_bits(),
                    corners: shape.corners.map(|shape| {
                        let [tl, tr, br, bl] = shape.radii.map(f32::to_bits);
                        [tl, tr, br, bl, shape.curve.to_bits()]
                    }),
                    transform: shape.transform.map(f32::to_bits),
                    mesh: shape.mesh.as_deref().and_then(|mesh| {
                        let key = match mesh.buffer.get()? {
                            batch::MeshBuffer::Resident(buffer) => {
                                MaskMeshKey::Resident(buffer.indices.clone())
                            }
                            batch::MeshBuffer::Streamed { id, .. } => MaskMeshKey::Streamed(*id),
                        };
                        Some((key, mesh.indices.clone()))
                    }),
                })
                .collect(),
        };
        if let Some(image) = self.compositor.masks.get(&key) {
            if target.view == self.main {
                self.main_inputs.read(image.view.clone());
            }
            coverage.mask = Some(image.clone());
            return Ok(coverage);
        }
        let mut intersection = None;
        let unclipped = Coverage::default();
        for shape in complex {
            let next = self.take_area(area, wgpu::TextureFormat::R8Unorm, 1);
            self.clear(&next.view);
            if let Some(mesh) = shape.mesh.as_deref() {
                self.meshes(&next, shape.transform, &unclipped, &[mesh])?;
            } else {
                let mut rect = compiler::solid_rect(shape.rect, [1.0; 4]);
                rect.radius = shape.radius;
                rect.corners = shape.corners;
                self.rects(&next, shape.transform, &unclipped, &[rect], &[], None)?;
            }
            self.finish(&next.view);
            if let Some(accumulated) = &intersection {
                // Draw the full logical area so zero coverage outside the new
                // shape also zeros the accumulated coverage.
                self.full(
                    &next,
                    None,
                    &unclipped,
                    accumulated,
                    area,
                    FullPass::MaskIntersect,
                )?;
                self.release(next);
            } else {
                intersection = Some(next);
            }
        }
        if let Some(image) = intersection {
            if self.compositor.masks.len() >= MASK_LIMIT
                && let Some(old) = self.compositor.masks.keys().next().cloned()
                && let Some(evicted) = self.compositor.masks.remove(&old)
            {
                self.release(evicted);
            }
            if target.view == self.main {
                self.main_inputs.read(image.view.clone());
            }
            self.compositor.masks.insert(key, image.clone());
            coverage.mask = Some(image);
        }
        Ok(coverage)
    }

    fn area(&self, bounds: PictureBounds, target: &Image) -> Option<[u32; 4]> {
        let target_area = self
            .damage_scissor
            .filter(|_| target.view == self.main)
            .unwrap_or_else(|| target.area());
        match bounds {
            PictureBounds::Empty => None,
            PictureBounds::Unbounded => Some(target_area),
            PictureBounds::Rect(bounds) => scissor(bounds, self.scene.scale, self.size)
                .and_then(|area| intersect_pixels(area, target_area)),
        }
    }

    fn fold_opacity(
        &mut self,
        group: Group,
        scopes: &[Scope],
        expected_groups: usize,
        target: &Image,
        origin: Affine,
        parent_clip: &Clip,
    ) -> Result<bool, Status> {
        if group.effect != Effect::Opacity || group.opacity <= 0.0 {
            return Ok(false);
        }
        let mut bounds = Vec::new();
        for scope in scopes.iter().filter(|scope| !scope.draws.is_empty()) {
            if scope.state.groups.len() != expected_groups {
                return Ok(false);
            }
            let transform = compiler::compose(origin, scope.state.transform);
            let margin = 1.0 / self.scene.scale;
            let mut add = |rect: [f32; 4], stroke: f32| {
                if bounds.len() >= 32 {
                    return false;
                }
                let PictureBounds::Rect(rect) = PictureBounds::rect(rect)
                    .expand(stroke)
                    .transform(transform)
                    .expand(margin)
                else {
                    return false;
                };
                if bounds.iter().any(|&previous| {
                    intersect(previous, rect)[2..]
                        .iter()
                        .all(|&size| size > 0.0)
                }) {
                    return false;
                }
                bounds.push(rect);
                true
            };
            for draw in &scope.draws {
                let compatible = match draw {
                    Draw::Rects(run) if run.blend == BlendKey::SrcOver => {
                        run.rects.iter().all(|rect| {
                            matches!(rect.kind, batch::RECT_FILL | batch::RECT_STROKE)
                                && add(rect.rect, rect.stroke_width * 0.5)
                        })
                    }
                    Draw::Images {
                        instances,
                        blend: BlendKey::SrcOver,
                        ..
                    } => instances.iter().all(|instance| add(instance.rect, 0.0)),
                    Draw::Glyphs(glyphs) => glyphs.iter().all(|glyph| {
                        let pad = glyph.size * 2.0 + 1.0;
                        glyph.gradient.is_none()
                            && add(
                                glyph.decoration.unwrap_or([
                                    glyph.x - pad,
                                    glyph.y - pad,
                                    2.0 * pad,
                                    2.0 * pad,
                                ]),
                                0.0,
                            )
                    }),
                    _ => false,
                };
                if !compatible {
                    return Ok(false);
                }
            }
        }
        for scope in scopes.iter().filter(|scope| !scope.draws.is_empty()) {
            let transform = compiler::compose(origin, scope.state.transform);
            let clip = merge_clip(parent_clip, &scope.state.clip, origin);
            let coverage = self.coverage(target, &clip)?;
            if coverage.empty {
                continue;
            }
            for draw in &scope.draws {
                match draw {
                    Draw::Rects(run) => {
                        let mut rects = run.rects.clone();
                        for rect in &mut rects {
                            rect.gradient_b[3] *= group.opacity;
                        }
                        self.rects(
                            target,
                            transform,
                            &coverage,
                            &rects,
                            run.stops(),
                            run.ramp.as_deref(),
                        )?;
                    }
                    Draw::Images {
                        source, instances, ..
                    } => {
                        if let Some(image) = self.scene.textures.get(source) {
                            let mut quads = instances.clone();
                            for quad in &mut quads {
                                quad.color = quad.color.map(|channel| channel * group.opacity);
                            }
                            self.images(
                                target,
                                transform,
                                &coverage,
                                &quads,
                                ImageSource::External(image),
                                BlendKey::SrcOver,
                            )?;
                        }
                    }
                    Draw::Glyphs(glyphs) => {
                        let mut quads = glyphs.clone();
                        for glyph in &mut quads {
                            glyph.color = glyph.color.map(|bits| {
                                half::f16::from_f32(
                                    half::f16::from_bits(bits).to_f32() * group.opacity,
                                )
                                .to_bits()
                            });
                        }
                        self.glyphs(target, transform, &coverage, &quads)?;
                    }
                    _ => return Err(Status::BadScene),
                }
            }
        }
        Ok(true)
    }

    fn track_opaque(&mut self, target: &Image, transform: Affine, clip: &Coverage, draw: &Draw) {
        if !self.gpu.caps.lcd || target.view != self.root {
            return;
        }
        if !draw.source_over() {
            self.opaque.clear();
            return;
        }
        if clip.mask.is_some() || clip.count > 0 {
            return;
        }
        let scale = self.scene.scale;
        let scissor = clip
            .scissor
            .unwrap_or_else(|| target.area())
            .map(|value| value as f32 / scale);
        let mut add = |rect| {
            let rect = intersect(rect, scissor);
            if rect[2] > 0.0 && rect[3] > 0.0 {
                if self.opaque.len() == 16 {
                    self.opaque.remove(0);
                }
                self.opaque.push(rect);
            }
        };
        match draw {
            Draw::Fill(color) if color[3] == 1.0 => add(scissor),
            Draw::Rects(run) if compiler::is_axis_aligned(transform) => {
                for rect in &run.rects {
                    if rect.color[3] != 1.0
                        || rect.flags != 0
                        || rect.kind != batch::RECT_FILL
                        || rect.corners.is_some()
                    {
                        continue;
                    }
                    let [x, y, w, h] = rect.rect;
                    let radius = rect.radius;
                    let interior = PictureBounds::rect([
                        x + radius,
                        y + radius,
                        w - 2.0 * radius,
                        h - 2.0 * radius,
                    ])
                    .transform(transform)
                    .expand(-1.0 / scale);
                    if let PictureBounds::Rect(interior) = interior {
                        add(interior);
                    }
                }
            }
            _ => {}
        }
    }

    fn draws(
        &mut self,
        layer: LayerId,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        draws: &[Draw],
    ) -> Result<(), Status> {
        let mut draws = draws.iter().peekable();
        while let Some(draw) = draws.next() {
            self.track_opaque(target, transform, clip, draw);
            match draw {
                Draw::Rects(run) => {
                    // Track adjacent runs before the iterator coalesces them.
                    if self.gpu.caps.lcd && target.view == self.root {
                        for next in draws.clone().take_while(
                            |draw| matches!(draw, Draw::Rects(next) if next.blend == run.blend),
                        ) {
                            self.track_opaque(target, transform, clip, next);
                        }
                    }
                    let blend = run.blend;
                    let mut first = Some(run);
                    let runs = std::iter::from_fn(|| {
                        if first.is_some() {
                            return first.take();
                        }
                        let Draw::Rects(next) = *draws.peek()? else {
                            return None;
                        };
                        if next.blend != blend {
                            return None;
                        }
                        draws.next();
                        Some(next)
                    });
                    if (self.compositor.early_z
                        || self.compositor.adaptive_early_z && run.rects.len() >= 64)
                        && blend == BlendKey::SrcOver
                    {
                        let runs: Vec<_> = runs.collect();
                        let batches: Vec<_> = runs
                            .iter()
                            .map(|run| (run.rects.as_slice(), run.stops()))
                            .collect();
                        let use_local = !self.global_depth(&target.view)
                            || batches.iter().any(|(rects, _)| {
                                rects.len() > 1 && rects.iter().any(|rect| rect.radius > 0.0)
                            });
                        if use_local && self.try_early_z(target, transform, clip, &batches)? {
                            continue;
                        }
                        self.prepared_rects(target, transform, clip, blend, runs)?;
                    } else {
                        self.prepared_rects(target, transform, clip, blend, runs)?;
                    }
                }
                Draw::Fill(color) => {
                    if clip.mask.is_none()
                        && clip.scissor.is_none()
                        && clip.count == 0
                        && if target.view == self.main {
                            self.main_pass.is_none()
                        } else {
                            self.pending.is_none()
                        }
                        && let Some(clear) = self.clears.get_mut(&target.view)
                        && (clear.a == 0.0 || color[3] == 1.0)
                    {
                        *clear = wgpu::Color {
                            r: color[0].into(),
                            g: color[1].into(),
                            b: color[2].into(),
                            a: color[3].into(),
                        };
                        continue;
                    }
                    let scale = self.scene.scale;
                    let rect = compiler::solid_rect(
                        [
                            0.0,
                            0.0,
                            self.size[0] as f32 / scale,
                            self.size[1] as f32 / scale,
                        ],
                        *color,
                    );
                    self.rects(target, IDENTITY, clip, &[rect], &[], None)?;
                }
                Draw::Glyphs(glyphs) => {
                    let next = std::iter::from_fn(|| {
                        if let Some(Draw::Glyphs(glyphs)) = draws.peek() {
                            let glyphs = glyphs.as_slice();
                            draws.next();
                            Some(glyphs)
                        } else {
                            None
                        }
                    });
                    self.glyphs(target, transform, clip, glyphs.iter().chain(next.flatten()))?;
                }
                Draw::Images {
                    source,
                    instances,
                    blend,
                } => {
                    use image_array::cacheable;
                    let use_array = self.gpu.image_array();
                    let use_atlas = !use_array && small_images::supports(self.gpu);
                    let spatial_images = self.compositor.spatial_batch
                        && self.compositor.pipelines.layouts.state.is_some()
                        && cacheable(source);
                    if spatial_images
                        && use_array
                        && self
                            .frame_image_array(target, transform, clip, *blend, source, instances)?
                    {
                        continue;
                    }
                    if spatial_images
                        && use_atlas
                        && self.scene.textures.get(source).is_some_and(|image| {
                            small_images::eligible_to_batch(
                                image,
                                instances,
                                transform,
                                self.scene.scale,
                            )
                        })
                    {
                        self.small_image_batch(
                            target,
                            transform,
                            clip,
                            *blend,
                            &[(source, instances)],
                        )?;
                        continue;
                    }
                    if (use_array || use_atlas)
                        && cacheable(source)
                        && matches!(draws.peek(), Some(Draw::Images { source: next, blend: next_blend, .. }) if next != source && next_blend == blend && cacheable(next))
                    {
                        let mut images = vec![(source.as_ref(), instances.as_slice())];
                        while let Some(Draw::Images {
                            source,
                            instances,
                            blend: next_blend,
                        }) = draws.peek()
                        {
                            if next_blend != blend || !cacheable(source) {
                                break;
                            }
                            images.push((source.as_ref(), instances.as_slice()));
                            draws.next();
                        }
                        if use_array {
                            self.image_array(target, transform, clip, *blend, &images)?;
                        } else {
                            self.small_image_batch(target, transform, clip, *blend, &images)?;
                        }
                        continue;
                    }
                    let next = std::iter::from_fn(|| {
                        if let Some(Draw::Images {
                            source: next_source,
                            instances,
                            blend: next_blend,
                        }) = draws.peek()
                            && next_source == source
                            && next_blend == blend
                        {
                            let instances = instances.as_slice();
                            draws.next();
                            Some(instances)
                        } else {
                            None
                        }
                    });
                    if let Some(image) = self.scene.textures.get(source) {
                        self.images(
                            target,
                            transform,
                            clip,
                            instances.iter().chain(next.flatten()),
                            ImageSource::External(image),
                            *blend,
                        )?;
                    }
                }
                Draw::ShaderEffect {
                    bounds,
                    shader,
                    uniforms,
                    textures,
                } => {
                    self.shader_effect(
                        target,
                        transform,
                        clip,
                        *bounds,
                        *shader,
                        uniforms,
                        textures,
                        self.scene.shader_uniforms.get(&layer),
                    )?;
                }
                Draw::Mesh(mesh) => {
                    let mut meshes = vec![mesh];
                    if mesh.composite == engine::scene::CanvasComposite::SourceOver {
                        while let Some(Draw::Mesh(next)) = draws.peek() {
                            if next.composite != mesh.composite {
                                break;
                            }
                            meshes.push(next);
                            draws.next();
                        }
                    }
                    self.meshes(target, transform, clip, &meshes)?;
                }
            }
        }
        Ok(())
    }

    fn rects(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        rects: &[RectInstance],
        stops: &[[f32; 4]],
        ramp: Option<&PreparedRamp>,
    ) -> Result<(), Status> {
        self.rect_batch_from_paint(
            target,
            transform,
            clip,
            BlendKey::SrcOver,
            (rects, stops, ramp),
        )
    }

    fn prepared_rects<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        runs: impl IntoIterator<Item = &'a compiler::RectRun>,
    ) -> Result<(), Status> {
        let view = self.rect_view(target, transform, clip, blend);
        let mut batch = Vec::new();
        let mut active: Option<PaintPage> = None;
        let mut addresses = Vec::new();
        for run in runs {
            let ramp = self
                .compositor
                .arena
                .lut
                .layer(self.gpu, run.ramp.as_deref());
            let stops = self.stops(run.stops())?;
            let empty = self.paint(stops, None)?;
            addresses.clear();
            addresses.resize(run.paints.len(), None);
            for &vertex in &run.vertices {
                let (page, index) = if vertex.paint == u32::MAX {
                    if active.is_some_and(|page| page.same_bindings(empty.0)) {
                        (active.unwrap_or(empty.0), u32::MAX)
                    } else {
                        empty
                    }
                } else {
                    let offset = vertex.paint as usize;
                    if let Some(address) = addresses[offset] {
                        address
                    } else {
                        let mut paint = run.paints[offset];
                        paint.gradient[1] = FrameArena::stop_index(stops);
                        paint.gradient[3] = ramp as f32;
                        let address = if self.gpu.caps.paint_storage {
                            self.compositor
                                .arena
                                .compiled_paint(self.gpu, stops, paint)?
                        } else {
                            self.paint(stops, Some(paint))?
                        };
                        addresses[offset] = Some(address);
                        address
                    }
                };
                if let Some(previous) = active
                    && !page.same_bindings(previous)
                {
                    self.rect_batch(target, clip, view, previous, &batch, blend)?;
                    batch.clear();
                }
                active = Some(page);
                batch.push(RectVertex {
                    paint: index,
                    ..vertex
                });
            }
        }
        if let Some(page) = active {
            self.rect_batch(target, clip, view, page, &batch, blend)?;
        }
        Ok(())
    }

    fn rect_view(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
    ) -> ViewUniform {
        let dither = self.gpu.dither_gradients
            && target.view == self.root
            && target.format.is_srgb()
            && blend == BlendKey::SrcOver;
        self.geometry_view(target, transform, clip, f32::from(dither))
    }

    fn rect_batch_from_paint(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        (rects, stops, ramp): (&[RectInstance], &[[f32; 4]], Option<&PreparedRamp>),
    ) -> Result<(), Status> {
        let view = self.rect_view(target, transform, clip, blend);
        let mut batch = Vec::new();
        let mut active = None;
        let ramp = self.compositor.arena.lut.layer(self.gpu, ramp);
        let stops = self.stops(stops)?;
        for rect in rects {
            let (page, index) =
                self.paint(stops, rect.extra(FrameArena::stop_index(stops), ramp))?;
            if let Some(previous) = active
                && !page.same_bindings(previous)
            {
                self.rect_batch(target, clip, view, previous, &batch, blend)?;
                batch.clear();
            }
            active = Some(page);
            batch.push(rect.vertex(index));
        }
        if let Some(page) = active {
            self.rect_batch(target, clip, view, page, &batch, blend)?;
        }
        Ok(())
    }

    fn try_early_z(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        batches: &[(&[RectInstance], &[[f32; 4]])],
    ) -> Result<bool, Status> {
        if target.view != self.root
            || clip.count != 0
            || clip.mask.is_some()
            || clip.scissor.is_some()
        {
            return Ok(false);
        }
        let [a, b, c, d, x, y] = transform;
        if b != 0.0 || c != 0.0 || a != d || a <= 0.0 {
            return Ok(false);
        }
        let scale = self.scene.scale;
        let count = batches.iter().map(|(rects, _)| rects.len()).sum::<usize>();
        let rects = || batches.iter().flat_map(|(rects, _)| rects.iter());
        let rounded = rects().any(|rect| rect.radius > 0.0);
        if !(16..=65535).contains(&count)
            || rects().any(|rect| {
                let [left, top, width, height] = rect.rect;
                rect.kind != batch::RECT_FILL
                    || rect.flags != 0
                    || !rect.radius.is_finite()
                    || rect.radius < 0.0
                    || rect.corners.is_some()
                    || rect.color[3] != 1.0
                    || rect.gradient_b[3] != 1.0
                    || ![
                        (left * a + x) * scale,
                        (top * a + y) * scale,
                        ((left + width) * a + x) * scale,
                        ((top + height) * a + y) * scale,
                    ]
                    .into_iter()
                    .all(|edge| edge.is_finite() && (rounded || edge == edge.round()))
            })
        {
            return Ok(false);
        }
        if !self.compositor.early_z {
            let mut bounds = PictureBounds::Empty;
            let mut area = 0.0;
            for rect in rects() {
                bounds = bounds.union(PictureBounds::rect(rect.rect));
                area += rect.rect[2] * rect.rect[3];
            }
            if !matches!(bounds, PictureBounds::Rect([_, _, w, h]) if area > 2.5 * w * h) {
                return Ok(false);
            }
        }
        let mut interiors = Vec::with_capacity(count);
        let mut edges = Vec::new();
        for (index, rect) in rects().enumerate() {
            let mut instance = rect.vertex(u32::MAX);
            instance.padding[0] = ((count - index) as f32 / (count + 1) as f32).to_bits();
            if rounded {
                edges.push(instance);
                let [x, y, w, h] = rect.rect;
                let radius = rect.radius.min(w.min(h) * 0.5);
                // Keep the depth write inside full SDF coverage, including the
                // antialiasing band at the diagonal of each rounded corner.
                let inset = radius * (1.0 - std::f32::consts::FRAC_1_SQRT_2) + 1.0 / (a * scale);
                if w <= 2.0 * inset || h <= 2.0 * inset {
                    continue;
                }
                instance.rect = [x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset];
            }
            interiors.push(instance);
        }
        if interiors.is_empty() {
            return Ok(false);
        }
        interiors.reverse();
        let view = self.transform(target, transform, clip)?;
        let globals = self.globals(view, Extra::None);
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            PipelineKind::EarlyZ,
            BlendKey::SrcOver,
        )?;
        let vertices = self.compositor.arena.vertices(self.gpu, &interiors)?;
        let edge_draw = if rounded {
            let pipeline = self.compositor.pipelines.get(
                self.gpu,
                single(target.format),
                PipelineKind::EarlyZEdges,
                BlendKey::SrcOver,
            )?;
            let vertices = self.compositor.arena.vertices(self.gpu, &edges)?;
            Some((pipeline, vertices))
        } else {
            None
        };
        self.flush_target(&target.view);
        let depth = self.take_exact_attachment(target.area(), pipelines::EARLY_DEPTH);
        let draws = 1 + u32::from(rounded);
        self.compositor.render_stats.draws += draws;
        let count = draw_count(interiors.len())?;
        self.compositor.render_stats.early_z_instances += count;
        let edges = edge_draw
            .map(|(pipeline, vertices)| Ok((pipeline, vertices, draw_count(edges.len())?)))
            .transpose()?;
        self.encode_pass(
            &target.view,
            RootSegment::EarlyZ(Box::new(EarlyZBatch {
                depth: depth.view.clone(),
                pipeline,
                globals,
                vertices,
                count,
                edges,
                mask: self.compositor.white_mask_group.clone(),
            })),
            GpuPassKind::EarlyZ,
            draws,
            None,
        );
        self.release(depth);
        Ok(true)
    }

    fn rect_batch(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: ViewUniform,
        page: PaintPage,
        vertices: &[RectVertex],
        blend: BlendKey,
    ) -> Result<(), Status> {
        let split_depth = self.global_depth(&target.view) && blend == BlendKey::SrcOver;
        let early_z = self.compositor.early_z;
        let runs = batch::rect_pipeline_runs(vertices).flat_map(|run| {
            let split =
                split_depth && (early_z || instances::large_opaque_core(run, &view, target.size));
            run.chunk_by(move |a, b| {
                !split || instances::same_depth_class(*a, *b, &view, target.size)
            })
        });
        for run in runs {
            let solid = self.compositor.specialize_paint
                && run
                    .iter()
                    .all(|rect| rect.shape[2] == 0.0 && rect.paint == u32::MAX);
            let kind = if batch::is_shadow(&run[0]) {
                shadow_pipeline(run)
            } else {
                self.rect_pipeline(target, clip, run, blend, solid)
            };
            self.read_mask(target, clip);
            let mask = self.mask_group(clip);
            let vertex_count = if batch::is_shadow(&run[0]) {
                self.gpu.shadow_vertices()
            } else {
                6
            };
            self.record_instances(
                target,
                view,
                clip.scissor,
                kind,
                blend,
                if solid {
                    Extra::None
                } else {
                    Extra::Paint(page)
                },
                mask,
                None,
                run,
                vertex_count,
            )?;
        }
        Ok(())
    }

    fn rect_pipeline(
        &self,
        target: &Image,
        clip: &Coverage,
        vertices: &[RectVertex],
        blend: BlendKey,
        solid: bool,
    ) -> PipelineKind {
        let unclipped = clip.count == 0 && clip.mask.is_none();
        if solid {
            return if unclipped {
                PipelineKind::RectSolidUnclipped
            } else {
                PipelineKind::RectSolid
            };
        }
        let dither = self.gpu.dither_gradients
            && target.view == self.root
            && target.format.is_srgb()
            && blend == BlendKey::SrcOver
            && vertices.iter().any(batch::has_gradient);
        match (unclipped, dither) {
            (false, false) => PipelineKind::Rect,
            (true, false) => PipelineKind::RectUnclipped,
            (false, true) => PipelineKind::RectDither,
            (true, true) => PipelineKind::RectDitherUnclipped,
        }
    }

    fn meshes(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        meshes: &[&Mesh],
    ) -> Result<(), Status> {
        if clip.count != 0
            || clip.mask.is_some()
            || !clip.stencil.is_empty()
            || target.format == wgpu::TextureFormat::R8Unorm
        {
            return self.meshes_msaa(target, transform, clip, meshes);
        }
        let mut pending = Vec::new();
        let mut pending_bounds = OverlapRows::new(target.size[1]);
        let mut view = None;
        let mut direct = None;
        let scale = self.scene.scale;
        let screen = |bounds: [f32; 4], padding: f32| {
            raster_bounds(
                PictureBounds::rect(bounds)
                    .transform(transform)
                    .expand(padding),
                target,
                scale,
            )
        };
        for &mesh in meshes {
            let geometry = if mesh.composite == engine::scene::CanvasComposite::SourceOver {
                mesh.fringe
                    .as_deref()
                    .and_then(|points| crate::fringe::mesh(points, transform, self.scene.scale))
            } else {
                None
            };
            let Some(geometry) = geometry else {
                pending.push(mesh);
                pending_bounds.insert(screen(mesh.bounds, 1.0 / scale));
                continue;
            };
            let bounds = screen(compiler::mesh_bounds(&geometry.vertices), 0.0);
            if pending_bounds.overlaps(bounds) {
                self.flush_direct_mesh(target, view, clip, direct.take())?;
                self.meshes_msaa(target, transform, clip, &pending)?;
                pending.clear();
                pending_bounds.clear();
            }
            let address = if let Some(address) = view {
                address
            } else {
                let address = self.transform(target, transform, clip)?;
                view = Some(address);
                address
            };
            if let Some((brush, combined)) = &mut direct
                && *brush == &mesh.brush
                && combined.vertices.len() + geometry.vertices.len()
                    <= FrameArena::vertex_limit::<[f32; 3]>(self.gpu)
                && combined.indices.len() + geometry.indices.len()
                    <= FrameArena::vertex_limit::<u32>(self.gpu)
            {
                let base = draw_count(combined.vertices.len())?;
                combined.vertices.extend(geometry.vertices);
                combined
                    .indices
                    .extend(geometry.indices.into_iter().map(|index| index + base));
            } else {
                self.flush_direct_mesh(target, Some(address), clip, direct.take())?;
                direct = Some((&mesh.brush, geometry));
            }
        }
        self.flush_direct_mesh(target, view, clip, direct)?;
        if !pending.is_empty() {
            self.meshes_msaa(target, transform, clip, &pending)?;
        }
        Ok(())
    }

    fn flush_direct_mesh(
        &mut self,
        target: &Image,
        view: Option<Address>,
        clip: &Coverage,
        batch: Option<(&batch::Brush, compiler::Tessellation)>,
    ) -> Result<(), Status> {
        let Some((brush, geometry)) = batch else {
            return Ok(());
        };
        let view = view.ok_or(Status::BadScene)?;
        let mut paint = compiler::solid_rect([0.0; 4], brush.solid());
        let (stops, ramp) = if let batch::Brush::Gradient(gradient) = brush {
            set_rect_gradient(&mut paint, gradient);
            (gradient.stops.as_slice(), gradient.ramp.as_deref())
        } else {
            (&[][..], None)
        };
        let ramp = self.compositor.arena.lut.layer(self.gpu, ramp);
        let stops = self.stops(stops)?;
        let (page, index) = self.paint(stops, paint.extra(FrameArena::stop_index(stops), ramp))?;
        let globals = self.globals(view, Extra::Paint(page));
        let material = self.mask_group(clip);
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            PipelineKind::Mesh,
            BlendKey::SrcOver,
        )?;
        let arena = &mut self.compositor.arena;
        let vertices = arena.vertices(self.gpu, &geometry.vertices)?;
        let indices = arena.vertices(self.gpu, &geometry.indices)?;
        self.compositor.render_stats.mesh_upload_bytes += vertices.size() + indices.size();
        let call = DirectMesh {
            pipeline,
            globals,
            material,
            geometry: batch::GpuMeshBuffer { vertices, indices },
            paint: arena.vertices(self.gpu, &[paint.vertex(index)])?,
            indices: draw_count(geometry.indices.len())?,
            scissor: clip.scissor.map(|area| target.local(area)),
        };
        let pass = self.pending_pass(target);
        pass.barrier(pass.calls.len() + 1);
        pass.calls.push(DrawCall::Mesh(call));
        pass.draws += 1;
        self.compositor.render_stats.draws += 1;
        Ok(())
    }

    fn meshes_msaa(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        meshes: &[&Mesh],
    ) -> Result<(), Status> {
        let mut bounds = PictureBounds::Empty;
        for mesh in meshes {
            bounds = bounds.union(PictureBounds::rect(mesh.bounds));
        }
        let Some(mut area) = self.area(bounds.transform(transform), target) else {
            return Ok(());
        };
        if let Some(clip_area) = clip.scissor {
            let Some(intersection) = intersect_pixels(area, clip_area) else {
                return Ok(());
            };
            area = intersection;
        }
        let blend =
            BlendKey::from_mode(batch::canvas_composite(meshes[0].composite)).unwrap_or_default();
        let mask_target = target.format == wgpu::TextureFormat::R8Unorm;
        let (format, resolved) = if mask_target {
            area = target.area();
            (target.format, target.clone())
        } else {
            let format = self.working;
            (format, self.take_area(area, format, 1))
        };
        let format_features = format.guaranteed_format_features(self.gpu.device.features());
        let samples = if format_features.flags.contains(
            wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        ) {
            4
        } else {
            1
        };
        let multisample = (samples > 1).then(|| self.take_attachment(area, format, samples));
        let view = self.transform(&resolved, transform, &Coverage::default())?;
        let mask_group = self.mask_group(&Coverage::default());
        let mut calls = Vec::with_capacity(meshes.len());
        let mut paints = Vec::with_capacity(meshes.len());
        for &mesh in meshes {
            if mesh.indices.is_empty() {
                continue;
            }
            let mut paint = compiler::solid_rect([0.0; 4], mesh.brush.solid());
            let (stops, ramp) = if let batch::Brush::Gradient(gradient) = &mesh.brush {
                set_rect_gradient(&mut paint, gradient);
                (gradient.stops.as_slice(), gradient.ramp.as_deref())
            } else {
                (&[][..], None)
            };
            let ramp = self.compositor.arena.lut.layer(self.gpu, ramp);
            let stops = self.stops(stops)?;
            let (page, index) =
                self.paint(stops, paint.extra(FrameArena::stop_index(stops), ramp))?;
            paints.push(paint.vertex(index));
            let geometry = self
                .compositor
                .arena
                .mesh(self.gpu, mesh.buffer.get().ok_or(Status::BadScene)?)?;
            calls.push((mesh, page, geometry));
        }
        let capacity = FrameArena::vertex_limit::<RectVertex>(self.gpu);
        let uploads = paints
            .chunks(capacity)
            .map(|chunk| self.compositor.arena.vertices(self.gpu, chunk))
            .collect::<Result<Vec<_>, _>>()?;
        let mut batches = Vec::new();
        for chunk in calls.chunks(capacity) {
            let mut groups = Vec::new();
            let mut first = 0;
            while let Some((_, page, _)) = chunk.get(first) {
                let run = chunk[first..]
                    .iter()
                    .take_while(|(_, next, _)| next.same_bindings(*page))
                    .count();
                groups.push((first, first + run, self.globals(view, Extra::Paint(*page))));
                first += run;
            }
            batches.push(groups);
        }
        self.flush_draws();
        if mask_target {
            self.clears.remove(&target.view);
        }
        self.count_pass(&resolved.view);
        let filled = calls
            .iter()
            .filter(|(mesh, _, _)| mesh.stencil_fill.is_some())
            .count();
        let stencil = (filled != 0 || !clip.stencil.is_empty())
            .then(|| self.take_attachment(area, pipelines::CANVAS_STENCIL, samples));
        let mut clip_triangles = Vec::with_capacity(clip.stencil.len());
        for shape in &clip.stencil {
            let mesh = shape.mesh.as_ref().ok_or(Status::BadScene)?;
            let view = self.transform(&resolved, shape.transform, &Coverage::default())?;
            let geometry = self
                .compositor
                .arena
                .mesh(self.gpu, mesh.buffer.get().ok_or(Status::BadScene)?)?;
            clip_triangles.push((mesh, self.globals(view, Extra::None), geometry));
        }
        let target_state = Target { format, samples };
        let clip_writer = if !clip_triangles.is_empty() {
            Some(self.compositor.pipelines.get(
                self.gpu,
                target_state,
                PipelineKind::StencilWrite,
                BlendKey::SrcOver,
            )?)
        } else {
            None
        };
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            target_state,
            if stencil.is_some() {
                PipelineKind::MeshStencil
            } else {
                PipelineKind::Mesh
            },
            BlendKey::SrcOver,
        )?;
        let mut winding = [None, None, None, None];
        for (mesh, _, _) in &calls {
            if let Some(fill) = &mesh.stencil_fill {
                let (index, kind) = stencil_writer(fill);
                if winding[index].is_none() {
                    winding[index] = Some(self.compositor.pipelines.get(
                        self.gpu,
                        target_state,
                        kind,
                        BlendKey::SrcOver,
                    )?);
                }
            }
        }
        let cover = (filled != 0)
            .then(|| {
                self.compositor.pipelines.get(
                    self.gpu,
                    target_state,
                    PipelineKind::StencilCover,
                    BlendKey::SrcOver,
                )
            })
            .transpose()?;
        let draw_total = draw_count(calls.len() + clip_triangles.len() + filled)?;
        self.compositor.render_stats.draws += draw_total;
        self.compositor.render_stats.stencil_draws += draw_count(clip_triangles.len() + filled)?;
        let mut pass = CanvasPass::new(&resolved, multisample.as_ref(), stencil.as_ref(), area);
        let kind = if mask_target {
            GpuPassKind::ClipMask
        } else if multisample.is_some() {
            GpuPassKind::CanvasMsaa
        } else {
            self.pass_kind(&resolved.view, format)
        };
        pass.stamps = pass_timestamps(&mut self.capture, kind, draw_total).map(PassStamps::from);
        {
            if let Some(writer) = &clip_writer {
                pass.pipeline(writer);
                for (reference, (mesh, globals, buffers)) in (0u32..).zip(&clip_triangles) {
                    pass.stencil(reference);
                    pass.globals(globals);
                    pass.geometry(buffers);
                    pass.draw(mesh.indices.clone(), 0);
                }
                pass.stencil(draw_count(clip_triangles.len())?);
            }
            pass.pipeline(&pipeline);
            pass.material(&mask_group);
            let mut last = None;
            let mut batch_bounds = OverlapRows::new(resolved.size[1]);
            for ((vertices, groups), chunk) in
                uploads.iter().zip(&batches).zip(calls.chunks(capacity))
            {
                pass.paint(vertices);
                for (first, end, globals) in groups {
                    pass.globals(globals);
                    let mut next = *first;
                    while next < *end {
                        let (mesh, _, _) = &chunk[next];
                        let Some(fill) = &mesh.stencil_fill else {
                            stencil_geometry(
                                &mut pass,
                                &mut last,
                                chunk,
                                next,
                                mesh.indices.clone(),
                            )?;
                            next += 1;
                            continue;
                        };
                        let rule = fill.rule;
                        let (index_of_rule, _) = stencil_writer(fill);
                        let write = winding[index_of_rule].as_ref().ok_or(Status::BadScene)?;
                        let cover = cover.as_ref().ok_or(Status::BadScene)?;
                        let screen = |mesh: &Mesh| {
                            raster_bounds(
                                PictureBounds::rect(mesh.bounds).transform(transform),
                                &resolved,
                                self.scene.scale,
                            )
                        };
                        batch_bounds.clear();
                        batch_bounds.insert(screen(mesh));
                        let mut end_of_batch = next + 1;
                        while end_of_batch < (*end).min(next + 64) {
                            let candidate = chunk[end_of_batch].0;
                            let bounds = screen(candidate);
                            if candidate.stencil_fill.as_ref().is_none_or(|candidate| {
                                candidate.rule != rule || candidate.analytic != fill.analytic
                            }) || batch_bounds.overlaps(bounds)
                            {
                                break;
                            }
                            batch_bounds.insert(bounds);
                            end_of_batch += 1;
                        }
                        self.compositor.render_stats.stencil_batches += 1;
                        pass.pipeline(write);
                        for index in next..end_of_batch {
                            stencil_geometry(
                                &mut pass,
                                &mut last,
                                chunk,
                                index,
                                chunk[index].0.indices.clone(),
                            )?;
                        }
                        pass.pipeline(cover);
                        for index in next..end_of_batch {
                            let range = chunk[index]
                                .0
                                .stencil_fill
                                .as_ref()
                                .ok_or(Status::BadScene)?
                                .cover_indices
                                .clone();
                            stencil_geometry(&mut pass, &mut last, chunk, index, range)?;
                        }
                        pass.pipeline(&pipeline);
                        next = end_of_batch;
                    }
                }
            }
        }
        let op = PlannedOperation::Canvas(Box::new(pass));
        if self.compositor.render_graph {
            let writes = std::iter::once(&resolved.texture)
                .chain(multisample.as_ref().map(|image| &image.texture))
                .chain(stencil.as_ref().map(|image| &image.texture))
                .cloned();
            self.graph.record(op, false, [], writes);
        } else {
            self.encode_operation(op);
        }
        if mask_target {
            if let Some(multisample) = multisample {
                self.release(multisample);
            }
            return Ok(());
        }
        let scale = self.scene.scale;
        self.images(
            target,
            IDENTITY,
            clip,
            &[ImageInstance {
                rect: area.map(|component| component as f32 / scale),
                uv: [
                    0.0,
                    0.0,
                    area[2] as f32 / resolved.size[0] as f32,
                    area[3] as f32 / resolved.size[1] as f32,
                ],
                color: [1.0; 4],
                flags: batch::IMAGE_NEAREST,
            }],
            ImageSource::Pooled(&resolved),
            blend,
        )?;
        if target.view == self.main {
            self.main_inputs.read(resolved.view.clone());
        } else {
            self.flush_draws();
        }
        self.release(resolved);
        if let Some(multisample) = multisample {
            self.release(multisample);
        }
        if let Some(stencil) = stencil {
            self.release(stencil);
        }
        Ok(())
    }

    fn images<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        images: impl IntoIterator<Item = &'a ImageInstance>,
        image: ImageSource<'_>,
        blend: BlendKey,
    ) -> Result<(), Status> {
        let view = self.geometry_view(target, transform, clip, 0.0);
        let mask = self.mask_group(clip);
        let kind = if image.nv12().is_some() {
            PipelineKind::ExternalImage
        } else if clip.count == 0 && clip.mask.is_none() {
            PipelineKind::ImageUnclipped
        } else {
            PipelineKind::Image
        };
        let mut vertices = Vec::new();
        let mut images = images.into_iter().peekable();
        while let Some(first) = images.next() {
            let nearest = first.flags & batch::IMAGE_NEAREST;
            vertices.push(
                first
                    .vertex(image.premultiplied_srgb())
                    .with_opacity_proof(image.opaque()),
            );
            while let Some(next) =
                images.next_if(|next| next.flags & batch::IMAGE_NEAREST == nearest)
            {
                vertices.push(
                    next.vertex(image.premultiplied_srgb())
                        .with_opacity_proof(image.opaque()),
                );
            }
            if let ImageSource::Pooled(input) = image {
                self.pending_pass(target).read(input);
            }
            self.read_mask(target, clip);
            let material = image.material(self, nearest != 0);
            self.record_instances(
                target,
                view,
                clip.scissor,
                kind,
                blend,
                Extra::None,
                material,
                Some(mask.clone()),
                &vertices,
                6,
            )?;
            vertices.clear();
        }
        Ok(())
    }

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
        let lcd = geometry.aligned
            && self.gpu.caps.lcd
            && target.format.is_srgb()
            && target.view == self.root
            && clip.count == 0
            && clip.mask.is_none()
            && !self.opaque.is_empty();
        let mut lcd_batch = false;
        for glyph in glyphs {
            let distance = if self.compositor.use_msdf
                && glyph.decoration.is_none()
                && geometry.distance_field(glyph.size)
            {
                self.compositor
                    .msdf
                    .glyph(
                        self.gpu,
                        self.scene.fonts,
                        &self.compositor.pipelines.layouts,
                        &self.compositor.linear,
                        glyph,
                    )
                    .map(|(vertex, created)| {
                        self.compositor.render_stats.bind_groups_created += u32::from(created);
                        vertex
                    })
            } else {
                None
            };
            if distance.is_none() && glyph.decoration.is_none() && geometry.vector(glyph.size) {
                let (mesh, uploaded) =
                    self.compositor
                        .outlines
                        .glyph(self.gpu, self.scene.fonts, glyph)?;
                if let Some(mut mesh) = mesh {
                    self.glyph_batch(target, clip, view, active, &batch, key)?;
                    batch.clear();
                    if uploaded > 0 {
                        self.compositor.render_stats.buffers_created += 2;
                        self.compositor.render_stats.mesh_upload_bytes += uploaded;
                    }
                    let scale = glyph.size / REFERENCE_PPEM;
                    mesh.brush = outline_brush(glyph, scale);
                    let matrix =
                        compiler::compose(transform, [scale, 0.0, 0.0, scale, glyph.x, glyph.y]);
                    self.meshes(target, matrix, clip, &[&mesh])?;
                    continue;
                }
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
            } else if let Some(vertex) = distance {
                (vertex, AtlasBinding::DistanceField, None)
            } else {
                let lcd = lcd && glyph.gradient.is_none();
                let quad = if lcd {
                    self.glyph_quad(glyph, geometry, true)?
                        .filter(|(vertex, _, placement)| {
                            if !placement.lcd {
                                return true;
                            }
                            let PictureBounds::Rect(rect) =
                                PictureBounds::rect(vertex.rect).transform(geometry.matrix)
                            else {
                                return false;
                            };
                            self.opaque
                                .iter()
                                .any(|&opaque| intersect(rect, opaque) == rect)
                        })
                } else {
                    None
                };
                let quad = match quad {
                    Some(quad) => Some(quad),
                    None => self.glyph_quad(glyph, geometry, false)?,
                };
                let Some((vertex, binding, placement)) = quad else {
                    continue;
                };
                (vertex, binding, Some(placement))
            };
            let color = placement.is_some_and(|placement| placement.color);
            let uses_lcd = placement.is_some_and(|placement| placement.lcd);
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
                        let ramp = self
                            .compositor
                            .arena
                            .lut
                            .layer(self.gpu, gradient.ramp.as_deref());
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
                                    ramp as f32,
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
            if key != binding || !active.same_bindings(page) || lcd_batch != uses_lcd {
                self.glyph_batch(target, clip, view, active, &batch, key)?;
                batch.clear();
            }
            key = binding;
            active = page;
            lcd_batch = uses_lcd;
            vertex.paint = index;
            batch.push(vertex);
        }
        self.glyph_batch(target, clip, view, active, &batch, key)
    }

    fn glyph_quad(
        &mut self,
        glyph: &compiler::GlyphDraw,
        geometry: GlyphTransform,
        lcd: bool,
    ) -> Result<Option<(GlyphVertex, AtlasBinding, crate::atlas::Placement)>, Status> {
        let position = geometry.position(glyph);
        let size = glyph.size * geometry.pixels;
        let Some(placement) =
            self.compositor
                .atlas
                .glyph(self.gpu, self.scene.fonts, glyph, size, position, lcd)
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
        if placement.lcd {
            flags |= batch::GLYPH_LCD;
        }
        if !placement.color && self.gpu.text_gamma && size <= 14.0 {
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
        let (material, created) = if key == AtlasBinding::DistanceField {
            (
                self.compositor.msdf.material().ok_or(Status::BadScene)?,
                false,
            )
        } else {
            self.compositor.atlas.material(
                self.gpu,
                &self.compositor.pipelines.layouts.atlas,
                &self.compositor.linear,
                key,
            )
        };
        self.compositor.render_stats.bind_groups_created += u32::from(created);
        self.read_mask(target, clip);
        let mask = self.mask_group(clip);
        let kind = if self.gpu.caps.lcd
            && glyphs
                .iter()
                .all(|glyph| glyph.flags_and_layer & batch::GLYPH_LCD != 0)
        {
            PipelineKind::TextLcd
        } else if self.compositor.specialize_paint
            && glyphs
                .iter()
                .all(|glyph| glyph.flags_and_layer & (7 | batch::GLYPH_MSDF) == 0)
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
            if matches!(kind, PipelineKind::TextMask | PipelineKind::TextLcd) {
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

    fn group(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Clip,
        group: Group,
        children: impl FnOnce(&mut Self, &Image) -> Result<(), Status>,
    ) -> Result<(), Status> {
        if group.opacity <= 0.0 {
            return Ok(());
        }
        let radius = group.sigma * self.scene.scale * compiler::effect_scale(transform);
        let valid_blur = radius.is_finite() && radius > 0.0;
        let mut group_clip = clip.clone();
        let corners = match group.effect {
            Effect::BackdropMaterial(_, corners) => corners,
            _ => None,
        };
        if let Some(bounds) = group.bounds {
            add_corner_clip(&mut group_clip, bounds, 0.0, corners, transform);
        }
        if let Effect::BackdropMaterial(material, _) = group.effect {
            if material.is_valid()
                && let Some(bounds) = group.bounds
                && let PictureBounds::Rect([_, top, _, height]) =
                    PictureBounds::rect(bounds).transform(transform)
            {
                let coverage = self.coverage(target, &group_clip)?;
                if !coverage.empty {
                    self.paint_backdrop(
                        target,
                        &coverage,
                        if valid_blur { radius } else { 0.0 },
                        Some(FullPass::BackdropMaterial {
                            material,
                            top: top * self.scene.scale,
                            inverse_height: 1.0 / (height * self.scene.scale),
                        }),
                    )?;
                }
            }
            return children(self, target);
        }
        if group.effect == Effect::BackdropBlur {
            if valid_blur {
                let coverage = self.coverage(target, &group_clip)?;
                if !coverage.empty {
                    self.paint_backdrop(target, &coverage, radius, None)?;
                }
            }
            return children(self, target);
        }

        let mut content = group.content;
        if let Some(rect) = group_clip.scissor {
            content = content.clip(rect);
        }
        let filter = group.effect == Effect::Blur && valid_blur;
        if filter {
            content = content.expand(3.0 * radius / self.scene.scale);
        }
        let Some(area) = self.area(content.expand(1.0 / self.scene.scale), target) else {
            return Ok(());
        };
        let source = self.take_area(area, self.working, 1);
        self.clear(&source.view);
        children(self, &source)?;
        self.finish(&source.view);
        let blurred = if filter {
            Some(self.blur(&source, radius, false)?)
        } else {
            None
        };
        if target.view == self.root
            && matches!(
                group.effect,
                Effect::Canvas(
                    engine::scene::CanvasComposite::DestinationOut
                        | engine::scene::CanvasComposite::Xor
                )
            )
        {
            self.opaque.clear();
        }
        let mode = match group.effect {
            Effect::Blend(mode) => batch::blend_mode(mode),
            Effect::Canvas(mode) => batch::canvas_composite(mode),
            _ => 0,
        };
        let blend = BlendKey::from_mode(mode);
        let snapshot = blend
            .is_none()
            .then(|| self.snapshot(target, area))
            .transpose()?;
        let coverage = if filter {
            self.coverage(target, &group_clip)?
        } else {
            Coverage::default()
        };
        if !coverage.empty {
            let visible = match coverage.scissor {
                Some(scissor) => intersect_pixels(scissor, area),
                None => Some(area),
            };
            if let Some(visible) = visible {
                self.full(
                    blurred.as_ref().unwrap_or(&source),
                    snapshot.as_ref(),
                    &coverage,
                    target,
                    visible,
                    if let Effect::ColorFilter(matrix) = group.effect {
                        FullPass::ColorFilter(matrix)
                    } else {
                        blend.map_or(
                            FullPass::Multiply {
                                opacity: group.opacity,
                            },
                            |blend| FullPass::Over {
                                blend,
                                opacity: group.opacity,
                            },
                        )
                    },
                )?;
            }
        }
        if let Some(snapshot) = snapshot {
            self.release(snapshot);
        }
        if let Some(blurred) = blurred {
            self.release(blurred);
        }
        self.release(source);
        Ok(())
    }

    fn snapshot(&mut self, target: &Image, area: [u32; 4]) -> Result<Image, Status> {
        if self.deferred_root.is_some() && target.view == self.root {
            return Err(Status::BadScene);
        }
        self.finish(&target.view);
        let snapshot = self.take_area(area, target.format, 1);
        let [x, y, w, h] = target.local(area);
        self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
        let op = PlannedOperation::Copy {
            source: target.texture.clone(),
            origin: wgpu::Origin3d { x, y, z: 0 },
            target: snapshot.texture.clone(),
            size: extent([w, h]),
        };
        if self.compositor.render_graph {
            self.graph.record(
                op,
                false,
                [target.texture.clone()],
                [snapshot.texture.clone()],
            );
        } else {
            self.encode_operation(op);
        }
        Ok(snapshot)
    }

    fn blur(&mut self, source: &Image, radius: f32, backdrop: bool) -> Result<Image, Status> {
        let mut levels = Vec::new();
        let mut size = source.logical;
        let mut remaining = radius;
        let origin = source.origin;
        let format = source.format;
        while remaining > 8.0 && levels.len() < 8 && size[0].max(size[1]) > 1 {
            size = size.map(|dim| dim.div_ceil(2));
            let down = self.take_area([origin[0], origin[1], size[0], size[1]], format, 1);
            let from = levels.last().unwrap_or(source);
            self.blur_pass(
                from,
                &down,
                FullPass::BlurDown {
                    radius: 1.0,
                    backdrop,
                },
            )?;
            levels.push(down);
            // Down/up sampling adds a small amount of variance.
            remaining = (remaining * remaining - 1.5).sqrt() * 0.5;
        }
        let input = levels.last().unwrap_or(source);
        // The apron fits sigma <= 8 at this pyramid level. The render path
        // is retained for larger kernels and for capabilities without storage.
        let [width, height] = input.logical;
        let hdr = format == wgpu::TextureFormat::Rgba16Float;
        let mut filtered = if (2.5..=8.0).contains(&remaining)
            && width.saturating_mul(height) >= 4096
            && let Some(pipeline) = self.compositor.compute_blur[usize::from(hdr)].clone()
        {
            self.compute_blur(&pipeline, input, remaining)?
        } else {
            let scratch = self.take_area(input.area(), format, 1);
            let image = self.take_area(input.area(), format, 1);
            self.blur_pass(
                input,
                &scratch,
                FullPass::Gaussian {
                    sigma: remaining,
                    vertical: false,
                    backdrop,
                },
            )?;
            self.blur_pass(
                &scratch,
                &image,
                FullPass::Gaussian {
                    sigma: remaining,
                    vertical: true,
                    backdrop,
                },
            )?;
            self.release(scratch);
            image
        };
        while let Some(level) = levels.pop() {
            let [w, h] = levels.last().map_or(source.logical, |image| image.logical);
            let next = self.take_area([origin[0], origin[1], w, h], format, 1);
            self.blur_pass(
                &filtered,
                &next,
                FullPass::BlurUp {
                    radius: 0.75,
                    backdrop,
                },
            )?;
            self.release(level);
            self.release(filtered);
            filtered = next;
        }
        Ok(filtered)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sigma is between 2.5 and 8 at the call site"
    )]
    fn compute_blur(
        &mut self,
        kernel: &ComputeBlur,
        source: &Image,
        sigma: f32,
    ) -> Result<Image, Status> {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        let scratch = self.take_area_with_usage(source.area(), kernel.format, 1, usage);
        let output = self.take_area_with_usage(source.area(), kernel.format, 1, usage);
        let [w, h] = source.logical;
        let params = self.uniform(&BlurParams {
            size: [w, h],
            radius: (sigma * 3.0).ceil() as u32,
            sigma,
        })?;
        let scratch_storage = scratch.storage_view.as_ref().unwrap_or(&scratch.view);
        let output_storage = output.storage_view.as_ref().unwrap_or(&output.view);
        for (axis, (input, target)) in [
            (&source.view, scratch_storage),
            (&scratch.view, output_storage),
        ]
        .into_iter()
        .enumerate()
        {
            let pipeline = kernel.pipeline(axis);
            let (buffer, offset) = self.compositor.arena.uniform_buffer(params);
            let key = (input.clone(), target.clone(), buffer.clone());
            let material = self.compositor.blur_groups.entry(key).or_insert_with(|| {
                self.compositor.render_stats.bind_groups_created += 1;
                pipelines::bind_group(
                    self.gpu,
                    &kernel.layout,
                    &[
                        wgpu::BindingResource::TextureView(input),
                        wgpu::BindingResource::TextureView(target),
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<BlurParams>() as u64),
                        }),
                    ],
                )
            });
            let stamps =
                pass_timestamps(&mut self.capture, GpuPassKind::ComputeBlur, 0).map(|writes| {
                    PassStamps {
                        queries: writes.query_set.clone(),
                        begin: writes.beginning_of_pass_write_index,
                        end: writes.end_of_pass_write_index,
                    }
                });
            let [tw, th] = if axis == 0 { [16, 8] } else { [8, 16] };
            let op = PlannedOperation::Compute {
                pipeline: pipeline.clone(),
                material: material.clone(),
                offset,
                stamps,
                groups: [w.div_ceil(tw), h.div_ceil(th)],
            };
            if self.compositor.render_graph {
                self.graph.record(
                    op,
                    false,
                    [input.texture().clone()],
                    [target.texture().clone()],
                );
            } else {
                self.encode_operation(op);
            }
            self.count_pass(target);
            self.compositor.render_stats.compute_passes += 1;
            self.compositor.render_stats.blur_pixels += u64::from(w) * u64::from(h);
        }
        self.release(scratch);
        Ok(output)
    }

    fn blur_pass(&mut self, source: &Image, to: &Image, kind: FullPass) -> Result<(), Status> {
        self.clear(&to.view);
        self.full(source, None, &Coverage::default(), to, to.area(), kind)
    }

    fn full(
        &mut self,
        source: &Image,
        backdrop: Option<&Image>,
        coverage: &Coverage,
        target: &Image,
        area: [u32; 4],
        kind: FullPass,
    ) -> Result<(), Status> {
        self.flush_draws();
        let view = self.uniform(&ViewUniform {
            transform: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, -2.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
            ..coverage.view(target, 1.0)
        })?;
        let value = match kind {
            FullPass::Over { opacity, .. } | FullPass::Multiply { opacity } => opacity,
            FullPass::BlurDown { radius, .. } | FullPass::BlurUp { radius, .. } => radius,
            FullPass::Gaussian { sigma, .. } => sigma,
            _ => 0.0,
        };
        let params = ParamsUniform {
            values: [
                source.origin[0] as f32 + source.sample_offset[0],
                source.origin[1] as f32 + source.sample_offset[1],
                value,
                f32::from(
                    matches!(kind, FullPass::Gaussian { vertical: true, .. })
                        || (target.view == self.root && self.straight_alpha),
                ),
            ],
            uv_bounds: source.uv_bounds(),
            backdrop_origin: backdrop.map_or([0.0; 4], |image| {
                [image.origin[0] as f32, image.origin[1] as f32, 0.0, 0.0]
            }),
            backdrop_bounds: backdrop.map_or([0.0; 4], Image::uv_bounds),
        };
        let immediates = self.gpu.effect_immediates();
        let extra = if immediates {
            Extra::None
        } else {
            Extra::Params(self.uniform(&params)?)
        };
        let globals = self.globals(view, extra);
        let input = ImageSource::Pooled(source).material(self, false);
        let composite = match kind {
            FullPass::Over { .. }
            | FullPass::Multiply { .. }
            | FullPass::Backdrop
            | FullPass::BackdropMaterial { .. } => {
                Some(self.composite_group(backdrop, coverage.mask.as_ref()))
            }
            _ => None,
        };
        let effect_values = match kind {
            FullPass::ColorFilter(matrix) => {
                let columns =
                    std::array::from_fn(|col| std::array::from_fn(|row| matrix[row][col]));
                Some((2, self.effect_values(&columns)?))
            }
            FullPass::BackdropMaterial {
                material,
                top,
                inverse_height,
            } => Some((
                3,
                self.effect_values(&[
                    material.tint,
                    [
                        material.saturation,
                        material.brightness,
                        material.taper,
                        inverse_height,
                    ],
                    [top, 0.0, 0.0, 0.0],
                    [0.0; 4],
                    [0.0; 4],
                ])?,
            )),
            _ => None,
        };
        self.compositor.render_stats.draws += 1;
        let blend = if let FullPass::Over { blend, .. } = kind {
            blend
        } else {
            BlendKey::SrcOver
        };
        let pipeline_kind = match kind {
            FullPass::DitherOutput if target.format.is_srgb() => PipelineKind::Dither,
            FullPass::DitherOutput => PipelineKind::DitherSrgb,
            FullPass::DisplayP3 if target.format.is_srgb() => PipelineKind::P3Linear,
            FullPass::DisplayP3 => PipelineKind::P3Encoded,
            FullPass::EncodeSrgb => PipelineKind::CopySrgb,
            FullPass::Copy => PipelineKind::Copy,
            FullPass::BlurDown { .. } => PipelineKind::BlurDown,
            FullPass::BlurUp { .. } => PipelineKind::BlurUp,
            FullPass::Gaussian { .. } => PipelineKind::Gaussian,
            FullPass::Over { .. } => PipelineKind::Over,
            FullPass::Multiply { .. } => PipelineKind::Multiply,
            FullPass::Backdrop => PipelineKind::Backdrop,
            FullPass::BackdropMaterial { .. } => PipelineKind::BackdropMaterial,
            FullPass::ColorFilter(_) => PipelineKind::ColorFilter,
            FullPass::MaskIntersect => PipelineKind::MaskIntersect,
        };
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), pipeline_kind, blend)?;
        let [x, y, w, h] = target.local(area);
        let kind = match kind {
            FullPass::BlurDown { backdrop, .. }
            | FullPass::BlurUp { backdrop, .. }
            | FullPass::Gaussian { backdrop, .. } => {
                self.compositor.render_stats.blur_pixels += u64::from(w) * u64::from(h);
                if backdrop {
                    GpuPassKind::BackdropBlur
                } else {
                    GpuPassKind::OffscreenEffect
                }
            }
            FullPass::Backdrop | FullPass::BackdropMaterial { .. } => GpuPassKind::BackdropBlur,
            FullPass::MaskIntersect => GpuPassKind::ClipMask,
            FullPass::Copy
            | FullPass::EncodeSrgb
            | FullPass::DisplayP3
            | FullPass::DitherOutput => {
                self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
                GpuPassKind::Copy
            }
            _ => GpuPassKind::OffscreenEffect,
        };
        let reads = std::iter::once(&source.texture)
            .chain(backdrop.map(|image| &image.texture))
            .chain(coverage.mask.as_ref().map(|image| &image.texture))
            .cloned()
            .collect::<Vec<_>>();
        let call = CompositeDraw {
            kind: pipeline_kind,
            blend,
            reads,
            pipeline,
            globals,
            source: input,
            composite,
            effect_values,
            immediates: immediates.then_some(params),
            viewport: [x, y, w, h],
        };
        if target.view == self.main {
            self.main_inputs.read(source.view.clone());
            if let Some(backdrop) = backdrop {
                self.main_inputs.read(backdrop.view.clone());
            }
            if let Some(mask) = &coverage.mask {
                self.main_inputs.read(mask.view.clone());
            }
            let pass = self.pending_pass(target);
            pass.draws += 1;
            pass.barrier(pass.calls.len() + 1);
            pass.reads.extend(call.reads.iter().cloned());
            pass.calls.push(DrawCall::Composite(call));
        } else {
            self.encode_pass(&target.view, RootSegment::Composite(call), kind, 1, None);
        }
        Ok(())
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

    fn mask_group(&mut self, coverage: &Coverage) -> wgpu::BindGroup {
        match &coverage.mask {
            Some(mask) => ImageSource::Pooled(mask).material(self, false),
            None => self.compositor.white_mask_group.clone(),
        }
    }

    fn composite_group(
        &mut self,
        backdrop: Option<&Image>,
        mask: Option<&Image>,
    ) -> wgpu::BindGroup {
        let compositor = &mut self.compositor;
        let view = |image: Option<&Image>| {
            image.map_or_else(|| compositor.white.clone(), |image| image.view.clone())
        };
        let key = (view(backdrop), view(mask));
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

    fn recycle(&mut self, image: Image, graph: bool) {
        let bytes = image.bytes();
        if bytes > POOL_BYTES {
            return;
        }
        while (!graph && self.images.len() >= POOL_LIMIT) || self.bytes + bytes > POOL_BYTES {
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

fn pass_timestamps<'a>(
    capture: &'a mut Option<(&mut Capture, Option<usize>)>,
    kind: GpuPassKind,
    draws: u32,
) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
    let (capture, slice) = capture.as_mut()?;
    capture.pass(kind, *slice, draws)
}

fn render<'a>(
    command: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) -> wgpu::RenderPass<'a> {
    command.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("impasto ordered pass"),
        timestamp_writes,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    })
}

fn outline_brush(glyph: &compiler::GlyphDraw, scale: f32) -> batch::Brush {
    let Some(gradient) = glyph.gradient.as_deref() else {
        return batch::Brush::Solid(glyph.color.map(|bits| half::f16::from_bits(bits).to_f32()));
    };
    let mut local = gradient.clone();
    local.a[0] = (local.a[0] - glyph.x) / scale;
    local.a[1] = (local.a[1] - glyph.y) / scale;
    match local.kind {
        0 | 3 => {
            local.a[2] = (local.a[2] - glyph.x) / scale;
            local.a[3] = (local.a[3] - glyph.y) / scale;
            local.radial_radii = local.radial_radii.map(|radius| radius / scale);
        }
        1 => local.a[2] /= scale,
        _ => {}
    }
    batch::Brush::Gradient(local)
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

fn shadow_pipeline(rects: &[RectVertex]) -> PipelineKind {
    if batch::has_shaped_shadow(rects) {
        PipelineKind::RectShadow
    } else {
        PipelineKind::RectCircularShadow
    }
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

/// Baked fill triangles can use stencil when the original path has only
/// orthogonal edges on pixel boundaries. MSAA coverage is then binary.
fn axis_aligned_path(path: &engine::scene::CanvasPath, transform: Affine, scale: f32) -> bool {
    use engine::scene::PathVerb;
    if transform[1] != 0.0 || transform[2] != 0.0 {
        return false;
    }
    let pixel = |x, y| {
        let (x, y) = compiler::apply(transform, x, y);
        [x * scale, y * scale]
    };
    let orthogonal = |a: [f32; 2], b: [f32; 2]| a[0] == b[0] || a[1] == b[1];
    let aligned = |p: [f32; 2]| p.into_iter().all(|v| v.is_finite() && v.round() == v);
    let mut contour: Option<([f32; 2], [f32; 2])> = None;
    for verb in path.verbs.iter() {
        match *verb {
            PathVerb::Move(x, y) => {
                if contour.is_some_and(|(first, end)| !orthogonal(first, end)) {
                    return false;
                }
                let point = pixel(x, y);
                if !aligned(point) {
                    return false;
                }
                contour = Some((point, point));
            }
            PathVerb::Line(x, y) => {
                let Some((first, last)) = contour else {
                    return false;
                };
                let point = pixel(x, y);
                if !aligned(point) || !orthogonal(last, point) {
                    return false;
                }
                contour = Some((first, point));
            }
            PathVerb::Close => {
                if contour.is_some_and(|(first, end)| !orthogonal(first, end)) {
                    return false;
                }
                contour = None;
            }
            _ => return false,
        }
    }
    contour.is_none_or(|(first, end)| orthogonal(first, end))
}

fn add_rect_clip(clip: &mut Clip, rect: [f32; 4], radius: f32, transform: Affine) {
    add_corner_clip(clip, rect, radius, None, transform);
}

fn add_corner_clip(
    clip: &mut Clip,
    rect: [f32; 4],
    radius: f32,
    corners: Option<engine::scene::CornerShape>,
    transform: Affine,
) {
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        clip.scissor = Some([0.0; 4]);
        return;
    }
    let aligned = transform[1].abs() < 0.0001 && transform[2].abs() < 0.0001;
    if aligned {
        let (x, y) = compiler::apply(transform, rect[0], rect[1]);
        let (right, bottom) = compiler::apply(transform, rect[0] + rect[2], rect[1] + rect[3]);
        let bounds = [
            x.min(right),
            y.min(bottom),
            (right - x).abs(),
            (bottom - y).abs(),
        ];
        clip.scissor = Some(
            clip.scissor
                .map_or(bounds, |ancestor| intersect(ancestor, bounds)),
        );
    }
    if radius > 0.0 || corners.is_some() || !aligned {
        clip.shapes.push(ClipShape {
            rect,
            radius,
            corners: corners.map(|shape| shape.fitted(rect[2], rect[3])),
            transform,
            mesh: None,
            path: None,
        });
    }
}

fn merge_clip(parent: &Clip, child: &Clip, transform: Affine) -> Clip {
    let mut result = parent.clone();
    if let Some(rect) = child.scissor {
        let transformed = if rect[2] <= 0.0 || rect[3] <= 0.0 {
            [0.0; 4]
        } else if let PictureBounds::Rect(bounds) = PictureBounds::rect(rect).transform(transform) {
            bounds
        } else {
            [0.0; 4]
        };
        result.scissor = Some(
            result
                .scissor
                .map_or(transformed, |old| intersect(old, transformed)),
        );
        if (transform[1].abs() >= 0.0001 || transform[2].abs() >= 0.0001)
            && let Some(exact) = child.hard_scissor
        {
            add_rect_clip(&mut result, exact, 0.0, transform);
        }
    }
    result
        .shapes
        .extend(child.shapes.iter().map(|shape| ClipShape {
            transform: compiler::compose(transform, shape.transform),
            ..shape.clone()
        }));
    result
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "all edges are clamped to the drawable"
)]
fn scissor(rect: [f32; 4], scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
    if rect[2] <= 0.0 || rect[3] <= 0.0 || !rect.iter().all(|x| x.is_finite()) {
        return None;
    }
    let x0 = (rect[0] * scale).floor().clamp(0.0, size[0] as f32) as u32;
    let y0 = (rect[1] * scale).floor().clamp(0.0, size[1] as f32) as u32;
    let x1 = ((rect[0] + rect[2]) * scale)
        .ceil()
        .clamp(0.0, size[0] as f32) as u32;
    let y1 = ((rect[1] + rect[3]) * scale)
        .ceil()
        .clamp(0.0, size[1] as f32) as u32;
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
}

fn set_scissor(pass: &mut wgpu::RenderPass<'_>, bounds: Option<[u32; 4]>) {
    if let Some([x, y, w, h]) = bounds {
        pass.set_scissor_rect(x, y, w, h);
    }
}

fn raster_bounds(bounds: PictureBounds, target: &Image, scale: f32) -> Bounds {
    match bounds {
        PictureBounds::Empty => Bounds([0.0; 4]),
        PictureBounds::Unbounded => Bounds([
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ]),
        PictureBounds::Rect([x, y, w, h]) => {
            let [ox, oy] = target.origin;
            Bounds([
                x * scale - ox as f32,
                y * scale - oy as f32,
                (x + w) * scale - ox as f32,
                (y + h) * scale - oy as f32,
            ])
        }
    }
}

fn stencil_writer(fill: &batch::StencilFill) -> (usize, PipelineKind) {
    use engine::scene::FillRule::{EvenOdd, NonZero};
    match (fill.rule, fill.analytic) {
        (NonZero, false) => (0, PipelineKind::StencilNonZero),
        (EvenOdd, false) => (1, PipelineKind::StencilEvenOdd),
        (NonZero, true) => (2, PipelineKind::StencilCurveNonZero),
        (EvenOdd, true) => (3, PipelineKind::StencilCurveEvenOdd),
    }
}

fn stencil_geometry<'a>(
    pass: &mut CanvasPass,
    last: &mut Option<&'a batch::GpuMeshBuffer>,
    chunk: &'a [(&Mesh, PaintPage, batch::GpuMeshBuffer)],
    index: usize,
    indices: std::ops::Range<u32>,
) -> Result<(), Status> {
    let (_, _, buffers) = &chunk[index];
    if *last != Some(buffers) {
        pass.geometry(buffers);
        *last = Some(buffers);
    }
    let index = draw_count(index)?;
    pass.draw(indices, index);
    Ok(())
}
