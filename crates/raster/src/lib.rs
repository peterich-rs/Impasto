//! wgpu raster and Parley typography for [`engine::scene::LayerTree`].

#[cfg(target_os = "android")]
mod android_vulkan;
mod arena;
mod atlas;
mod batch;
mod batcher;
mod color;
mod compiler;
mod compositor;
mod compositor_tracks;
mod compute_blur;
mod damage;
mod fonts;
mod fringe;
mod gpu;
mod gpu_view;
mod gradient_lut;
mod graph;
mod image_upload;
mod inline_image;
mod msdf;
mod outline;
mod pipeline_disk_cache;
mod pipelines;
mod prepare;
mod profiler;
mod resident;
mod runtime_effect;
mod shaper;
mod state_table;
mod status;
mod text_index;
mod textures;

pub use damage::DamageRect;
pub use fonts::{FontLibrary, ShapeStats};
pub use gpu_view::{GpuContext, GpuViewError, GpuViewFormat, GpuViewId, GpuViewProducer};
pub use image_upload::ImageUploadStats;
pub use profiler::{
    GpuFrameTiming, GpuPassKind, GpuPassSource, GpuPassTiming, GpuSample, GpuTimingStatus,
    GpuTimingUpdate,
};
pub use runtime_effect::EffectFailure;
pub use shaper::ParleyShaper;
pub use status::{RasterBackend, RasterOptions, Status, ValidationMode};
pub use wgpu;

/// Drawable size in physical pixels. A zero axis means the size is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_usable(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// RGBA pixels borrowed until [`crate::Rasterizer::submit_layers`] returns.
pub struct ImagePixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

#[cfg(target_os = "ios")]
mod metal;
mod surface;

pub use atlas::AtlasStats;
pub use compositor::{RenderStats, SubtreeCacheStats};
#[cfg(target_os = "ios")]
pub use metal::MetalSliceBatch;
pub use surface::{RawSurfaceHandle, WgpuSurface};
pub use textures::{GpuImage, TextureFactory};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use engine::scene::{LayerId, LayerKind, LayerTree, PictureId};

use compiler::{Affine, CompileContext, CompiledPicture, IDENTITY, compose, layer_transform};
use compositor::{Compositor, GlyphScene, Scene};
use gpu::Gpu;
use profiler::{Capture, GpuProfiler};
use textures::TextureStore;

static NEXT_DEVICE: AtomicU64 = AtomicU64::new(1);

/// Counts from scene submission and the most recent encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmitStats {
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub picture_workers: u32,
    pub damage: DamageRect,
    pub layer_count: u32,
    pub picture_prepare: Duration,
    pub compose: Duration,
    pub encode: Duration,
    pub flush: Duration,
    pub queue_submit: Duration,
    pub submit_done: Option<Instant>,
    pub acquire: Duration,
    pub surface_present: Duration,
    pub pictures: u32,
    pub textures: u32,
    pub atlas: AtlasStats,
    pub image_upload: ImageUploadStats,
    pub render: RenderStats,
}

#[derive(Clone, Debug)]
pub struct SubmitInfo {
    pub stats: SubmitStats,
    pub platform_views: Arc<[u64]>,
}

pub struct Rasterizer {
    options: RasterOptions,
    fonts: FontLibrary,
    gpu_views: gpu_view::GpuViews,
    render_time_ms: f64,
    first_render_ms: Option<f64>,
    device: Option<DeviceState>,
    frame: Option<Frame>,
    stats: SubmitStats,
    reuse_pictures: bool,
    parallel_prepare: bool,
    late_acquire: bool,
    pipeline_cache_dir: Option<std::path::PathBuf>,
    prepared_buffers: u32,
    prepared_mesh_bytes: u64,
    gpu_frame: Option<u64>,
    gpu_sample: GpuSample,
    #[cfg(target_os = "ios")]
    frame_number: u64,
}

struct DeviceState {
    gpu: Gpu,
    generation: u64,
    compositor: Compositor,
    profiler: Option<GpuProfiler>,
    textures: TextureStore,
}

#[derive(Default)]
struct FrameWork {
    capture: Option<Capture>,
    producers: Vec<wgpu::CommandBuffer>,
    encode_elapsed: Duration,
    prepare_elapsed: Duration,
}

struct PreparedSurfaceFrame {
    work: FrameWork,
    output: compositor::PreparedOutput,
}

type EncodedFrame = (Vec<wgpu::CommandBuffer>, wgpu::CommandEncoder);

struct Frame {
    tree: Arc<LayerTree>,
    order: Vec<LayerId>,
    scale: f32,
    pictures: HashMap<PictureId, Arc<CompiledPicture>>,
    views: Arc<[u64]>,
    gpu_view_requests: gpu_view::Requests,
    shader_uniforms: runtime_effect::ShaderUniformOverrides,
    damage: DamageRect,
    complex_damage: bool,
    pending_images: bool,
    has_images: bool,
    has_glyphs: bool,
    revision: u64,
    previous_revision: u64,
    scene_revision: u64,
    #[cfg(target_os = "ios")]
    slices: Vec<HashSet<LayerId>>,
}

impl Rasterizer {
    /// Defers adapter selection until the first surface is attached.
    pub fn create(options: RasterOptions, fonts: FontLibrary) -> Result<Self, Status> {
        Ok(Self {
            options,
            fonts,
            gpu_views: gpu_view::GpuViews::default(),
            render_time_ms: 0.0,
            first_render_ms: None,
            device: None,
            frame: None,
            stats: SubmitStats::default(),
            reuse_pictures: false,
            parallel_prepare: std::env::var("IMPASTO_PARALLEL_PREPARE").as_deref() != Ok("0"),
            late_acquire: std::env::var("IMPASTO_LATE_ACQUIRE").as_deref() != Ok("0"),
            pipeline_cache_dir: None,
            prepared_buffers: 0,
            prepared_mesh_bytes: 0,
            gpu_frame: None,
            gpu_sample: GpuSample::default(),
            #[cfg(target_os = "ios")]
            frame_number: 0,
        })
    }

    pub fn headless(options: RasterOptions, fonts: FontLibrary) -> Result<Self, Status> {
        let mut raster = Self::create(options, fonts)?;
        let mut last = Status::NotReady;
        for backend in raster.backends() {
            match Gpu::request(Gpu::instance(backend), backend, options, None, None).and_then(
                |gpu| {
                    let generation = NEXT_DEVICE.fetch_add(1, Ordering::Relaxed);
                    Self::new_device(gpu, generation, None)
                },
            ) {
                Ok(device) => {
                    raster.device = Some(device);
                    return Ok(raster);
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    fn backends(&self) -> impl Iterator<Item = RasterBackend> + use<> {
        self.options
            .backend
            .into_iter()
            .chain(RasterBackend::candidates().iter().copied().filter({
                let automatic = self.options.backend.is_none();
                move |_| automatic
            }))
    }

    fn new_device(
        gpu: Gpu,
        generation: u64,
        surface: Option<wgpu::TextureFormat>,
    ) -> Result<DeviceState, Status> {
        let compositor = gpu.validation(|| Compositor::new(&gpu, surface))?;
        let textures = TextureStore::new(generation);
        Ok(DeviceState {
            gpu,
            generation,
            compositor,
            profiler: None,
            textures,
        })
    }

    /// Selects an app-private Vulkan pipeline cache directory before surface attachment.
    ///
    /// # Safety
    ///
    /// The directory must be writable solely by the host application and its
    /// files must come from this application. Android's Context.getCacheDir meets this contract.
    #[expect(
        unsafe_code,
        reason = "wgpu startup trusts opaque data produced by the application"
    )]
    pub unsafe fn set_pipeline_cache_directory(&mut self, directory: std::path::PathBuf) {
        self.pipeline_cache_dir = Some(directory);
    }

    /// Configures timestamp capability before the first device is created.
    pub fn enable_gpu_timing(&mut self) {
        self.options.enable_gpu_timing = true;
    }

    /// Registers a same-device view producer on the Raster queue.
    ///
    /// # Errors
    /// Fails if this producer ID is in use or producer capacity is exhausted.
    pub fn register_gpu_view(
        &mut self,
        id: GpuViewId,
        producer: impl GpuViewProducer,
    ) -> Result<(), GpuViewError> {
        self.gpu_views.register(id, producer)?;
        self.invalidate_pictures();
        Ok(())
    }

    /// Reports whether the submitted scene references a registered same-device producer.
    pub fn has_registered_gpu_views_in_scene(&self) -> bool {
        self.frame
            .as_ref()
            .is_some_and(|frame| !frame.gpu_view_requests.is_empty())
    }

    /// Updates the target timestamp used by runtime effects and native producers.
    pub fn set_render_time_ms(&mut self, target_ms: f64) {
        if target_ms.is_finite() && target_ms >= 0.0 {
            let first = *self.first_render_ms.get_or_insert(target_ms);
            self.render_time_ms = target_ms.max(first).max(self.render_time_ms);
        }
    }

    /// Reads new shader diagnostics from the current device once.
    pub fn take_effect_failures(&mut self) -> Vec<EffectFailure> {
        self.device
            .as_mut()
            .map_or_else(Vec::new, |device| device.compositor.take_effect_failures())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "WGSL accepts a single-precision relative clock"
    )]
    fn render_gpu_views(
        &mut self,
        capture: Option<&mut Capture>,
    ) -> Result<Vec<wgpu::CommandBuffer>, Status> {
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        let frame = self.frame.as_ref().ok_or(Status::NotReady)?;
        let time = self.render_time_ms;
        device.compositor.effect_seconds = self.first_render_ms.map_or(0.0, |start| {
            ((time - start) / 1000.0).min(f64::from(f32::MAX)) as f32
        });
        let mut commands = Vec::new();
        if !frame.gpu_view_requests.is_empty() || device.textures.has_gpu_views() {
            device.gpu.check_async_health()?;
            let views = self.gpu_views.render(
                &device.gpu,
                device.generation,
                &frame.gpu_view_requests,
                time,
                capture,
                &mut commands,
            );
            device.textures.set_gpu_views(views);
        }
        Ok(commands)
    }

    fn discard_device(&mut self) {
        self.gpu_views.device_lost();
        self.device = None;
        self.frame = None;
        self.invalidate_pictures();
    }

    /// Arms one present attempt. With no ID the attempt skips GPU queries.
    pub fn begin_gpu_frame(&mut self, frame_id: Option<u64>) {
        self.gpu_frame = frame_id;
        self.gpu_sample = GpuSample::default();
        self.stats.acquire = Duration::ZERO;
        self.stats.surface_present = Duration::ZERO;
        self.stats.submit_done = None;
    }

    pub fn gpu_sample(&self) -> GpuSample {
        self.gpu_sample
    }

    pub fn gpu_timings_pending(&self) -> bool {
        self.device
            .as_ref()
            .and_then(|device| device.profiler.as_ref())
            .is_some_and(GpuProfiler::has_pending)
    }

    pub fn drain_gpu_timings(&mut self) -> Vec<GpuTimingUpdate> {
        let Some(device) = self.device.as_mut() else {
            return Vec::new();
        };
        device
            .profiler
            .as_mut()
            .map_or_else(Vec::new, |profiler| profiler.drain(&device.gpu))
    }

    /// Completes pending samples as stale before the host replaces the device.
    pub fn retire_gpu_timings(&mut self) -> Vec<GpuTimingUpdate> {
        self.device
            .as_mut()
            .and_then(|device| device.profiler.take())
            .map_or_else(Vec::new, |mut profiler| profiler.retire())
    }

    fn begin_gpu_capture(&mut self) -> Option<Capture> {
        let frame_id = self.gpu_frame.take()?;
        let device = self.device.as_mut()?;
        let sample = GpuSample {
            frame_id,
            device_generation: device.generation,
            status: GpuTimingStatus::Unsupported,
        };
        self.gpu_sample = sample;
        if device.gpu.caps.has_timestamp_query && device.profiler.is_none() {
            match device.gpu.validation(|| Ok(GpuProfiler::new(&device.gpu))) {
                Ok(profiler) => device.profiler = profiler,
                Err(_) => {
                    self.gpu_sample.status = GpuTimingStatus::ReadbackFailed;
                    return None;
                }
            }
        }
        let profiler = device.profiler.as_ref()?;
        self.gpu_sample.status = GpuTimingStatus::DroppedSlotFull;
        let capture = profiler.begin(sample)?;
        self.gpu_sample.status = GpuTimingStatus::ReadbackFailed;
        Some(capture)
    }

    fn finish_render_stats(&mut self, mut stats: RenderStats) {
        stats.buffers_created += std::mem::take(&mut self.prepared_buffers);
        stats.mesh_upload_bytes += std::mem::take(&mut self.prepared_mesh_bytes);
        self.stats.render = stats;
    }

    pub fn texture_factory(&self) -> Option<TextureFactory> {
        let device = self.device.as_ref()?;
        (!device.gpu.is_lost()).then(|| TextureFactory::new(&device.gpu, device.generation))
    }

    pub fn install_uploaded(&mut self, source: &str, image: &GpuImage) {
        if let Some(device) = self.device.as_mut() {
            device.textures.install(source, image.clone());
        }
    }

    pub fn forget_texture(&mut self, source: &str) {
        if let Some(device) = self.device.as_mut() {
            device.textures.forget(source);
            device.compositor.evict_images(device.textures.evictions());
        }
    }

    /// Clears the working image cache when the host changes decode scale.
    /// The installed frame keeps using its previous textures until resubmission.
    pub fn invalidate_textures(&mut self) {
        if let Some(device) = self.device.as_mut() {
            device.textures.clear();
            device.compositor.evict_images(device.textures.evictions());
        }
    }

    /// The next submit compiles a new scene stream. The installed frame remains presentable.
    pub fn invalidate_pictures(&mut self) {
        self.reuse_pictures = false;
    }

    pub fn trim(&mut self) {
        self.invalidate_pictures();
        if let Some(device) = self.device.as_mut() {
            device.textures.clear();
            drop(device.textures.evictions());
            device.compositor.trim();
        }
        self.fonts.sweep();
    }

    pub fn submit_stats(&self) -> SubmitStats {
        self.stats
    }

    pub fn platform_views(&self) -> &[u64] {
        self.frame.as_ref().map_or(&[], |frame| &frame.views)
    }

    /// Validates and installs a complete scene. On failure the previous frame stays intact.
    pub fn submit_layers<'pixels>(
        &mut self,
        tree: &LayerTree,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        self.submit_shared_layers(Arc::new(tree.clone()), point_scale, resolve)
    }

    /// Installs a scene already shared by the UI and raster queues.
    pub fn submit_shared_layers<'pixels>(
        &mut self,
        tree: Arc<LayerTree>,
        point_scale: f32,
        resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        self.submit_scene(tree, point_scale, None, resolve)
    }

    /// Installs a checkpoint with an optional incremental damage description.
    pub fn submit_scene<'pixels>(
        &mut self,
        tree: Arc<LayerTree>,
        point_scale: f32,
        delta: Option<&engine::scene::SceneDelta>,
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        if !point_scale.is_finite() || point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        let started = Instant::now();
        let _prepare_span =
            tracing::debug_span!(target: "impasto::render", "prepare_pictures").entered();
        let order = paint_order(&tree)?;
        let scales = picture_scales(&tree, &order, point_scale)?;
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let mut textures = device.textures.begin_frame();
        let previous = self.frame.as_ref().filter(|_| self.reuse_pictures);
        let previous_revision = self.frame.as_ref().map_or(0, |frame| frame.revision);
        let compile_scale = |picture| scales.get(&picture).copied().unwrap_or(point_scale);
        let (pool, mut prepared) = if self.parallel_prepare {
            prepare::pictures(
                tree.pictures.iter().filter_map(|picture| {
                    let scale = compile_scale(picture.id);
                    let cached = previous
                        .and_then(|frame| frame.pictures.get(&picture.id))
                        .is_some_and(|old| {
                            old.device == device.generation
                                && old.generation == picture.generation
                                && old.scale == scale
                                && old.origin == picture.origin
                                && old
                                    .images
                                    .iter()
                                    .all(|(source, _)| textures.same_geometry(source))
                                && old.waiting.is_empty()
                        });
                    (!cached).then_some((picture.as_ref(), scale))
                }),
                |source| {
                    textures
                        .get(source)
                        .map(|image| (image.width(), image.height()))
                },
                device.gpu.analytic_canvas_curves(),
            )
        } else {
            (None, HashMap::new())
        };
        let mut stats = SubmitStats {
            picture_workers: pool.map_or(0, |pool| {
                u32::try_from(pool.current_num_threads()).unwrap_or(u32::MAX)
            }),
            ..SubmitStats::default()
        };
        let mut pictures = HashMap::with_capacity(tree.pictures.len());
        let mut has_glyphs = false;
        let mut pending_images = false;
        let mut has_images = false;
        let mut rebuilt_atlas_pictures = HashSet::new();
        for picture in &tree.pictures {
            let compile_scale = compile_scale(picture.id);
            let cached = previous
                .and_then(|frame| frame.pictures.get(&picture.id))
                .filter(|cached| {
                    cached.device == device.generation
                        && cached.generation == picture.generation
                        && cached.scale == compile_scale
                        && cached.origin == picture.origin
                        && cached
                            .images
                            .iter()
                            .all(|(source, _)| textures.same_geometry(source))
                        && cached.waiting.iter().all(|source| {
                            textures.get(source).is_none() && resolve(source).is_none()
                        })
                });
            let compiled = if let Some(cached) = cached {
                for (source, _) in &cached.images {
                    textures.touch(source);
                }
                stats.picture_reused = stats.picture_reused.saturating_add(1);
                Arc::clone(cached)
            } else {
                let _compile_span = tracing::trace_span!(
                    target: "impasto::render",
                    "compile_picture",
                    id = picture.id.0
                )
                .entered();
                let stream_mesh = previous
                    .and_then(|frame| frame.pictures.get(&picture.id))
                    .is_some_and(|old| old.generation != picture.generation);
                let mut context = CompileContext {
                    gpu: &device.gpu,
                    device_generation: device.generation,
                    textures: &mut textures,
                    resolve: &mut resolve,
                    point_scale: compile_scale,
                    stream_mesh,
                };
                let result = if let Some(prepared) = prepared.remove(&picture.id) {
                    let compiled = prepared?.install(
                        context.gpu,
                        context.device_generation,
                        context.stream_mesh,
                    )?;
                    for (source, _) in picture.images() {
                        context.textures.touch(source);
                    }
                    compiled
                } else {
                    compiler::compile(picture, &mut context, pool)?
                };
                stats.picture_rebuilt = stats.picture_rebuilt.saturating_add(1);
                if self.parallel_prepare && (result.has_glyphs || result.has_vector_fields) {
                    rebuilt_atlas_pictures.insert(picture.id);
                }
                if let Some(mesh) = result
                    .mesh_buffer
                    .get()
                    .and_then(batch::MeshBuffer::resident)
                {
                    stats.render.buffers_created += 2;
                    stats.render.mesh_upload_bytes += mesh.vertices.size() + mesh.indices.size();
                }
                Arc::new(result)
            };
            has_glyphs |= compiled.has_glyphs;
            pending_images |= !compiled.waiting.is_empty();
            has_images |= !picture.images().is_empty();
            if pictures.insert(picture.id, compiled).is_some() {
                return Err(Status::BadScene);
            }
        }
        let mut views = Vec::new();
        #[cfg(target_os = "ios")]
        let mut slices = vec![HashSet::new()];
        for &id in &order {
            let layer = &tree.layers[id.0 as usize];
            match layer.kind {
                LayerKind::PlatformView { node } => {
                    views.push(node);
                    #[cfg(target_os = "ios")]
                    slices.push(HashSet::new());
                }
                LayerKind::Picture { picture } if !pictures.contains_key(&picture) => {
                    return Err(Status::BadScene);
                }
                #[cfg(target_os = "ios")]
                LayerKind::Picture { .. } | LayerKind::Fill { .. } | LayerKind::Paint(_) => {
                    if let Some(slice) = slices.last_mut() {
                        let mut ancestor = Some(id);
                        while let Some(parent) = ancestor {
                            if !slice.insert(parent) {
                                break;
                            }
                            ancestor = tree.layers[parent.0 as usize].parent;
                        }
                    }
                }
                _ => {}
            }
        }
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let platform_views: Arc<[u64]> = views.into();
        let gpu_view_requests = self.gpu_views.requests(&tree, &scales, point_scale);
        let complex_damage = !platform_views.is_empty()
            || pictures
                .values()
                .any(|picture| picture.has_shader_effect || picture.reads_root)
            || tree.layers.iter().any(|layer| {
                matches!(
                    layer.kind,
                    LayerKind::BackdropBlur { .. }
                        | LayerKind::BackdropMaterial { .. }
                        | LayerKind::Blend {
                            mode: engine::scene::BlendMode::Multiply,
                            ..
                        }
                )
            });
        let mut frame = Frame {
            shader_uniforms: tree.shader_uniforms.clone(),
            tree,
            order,
            scale: point_scale,
            pictures,
            views: Arc::clone(&platform_views),
            gpu_view_requests,
            damage: DamageRect::Full,
            complex_damage,
            pending_images,
            has_images,
            has_glyphs,
            revision: previous_revision.wrapping_add(1),
            previous_revision,
            scene_revision: previous_revision.wrapping_add(1),
            #[cfg(target_os = "ios")]
            slices,
        };
        let gpu = &device.gpu;
        let install = || {
            let damage = match (previous, delta) {
                (Some(old), Some(delta))
                    if old.tree.epoch == delta.base_epoch
                        && old.scale == point_scale
                        && !old.has_images
                        && !frame.has_images =>
                {
                    damage::changed_layers(
                        old,
                        &frame.tree,
                        |id| frame.pictures.get(&id).map(Arc::as_ref),
                        frame.complex_damage,
                        delta,
                    )
                }
                _ => damage::between(previous, &frame, &textures),
            };
            let upload = textures.commit(gpu)?;
            Ok((damage, upload))
        };
        let (damage, upload) = if !rebuilt_atlas_pictures.is_empty()
            && let Some(pool) = prepare::pool()
        {
            let compositor = &mut device.compositor;
            let glyph_scene = GlyphScene {
                tree: &frame.tree,
                pictures: &frame.pictures,
                fonts: &self.fonts,
                scale: frame.scale,
            };
            // Installed picture geometry determines the exact glyph keys.
            // Its CPU prefetch can overlap damage and texture submission.
            pool.in_place_scope(|scope| {
                scope.spawn(|_| {
                    compositor.prepare_glyphs(pool, gpu, glyph_scene, &rebuilt_atlas_pictures)
                });
                install()
            })
        } else {
            install()
        }?;
        frame.damage = damage;
        stats.damage = damage;
        stats.image_upload = upload;
        device.compositor.evict_images(device.textures.evictions());
        device.compositor.prune_subtrees(&frame.pictures);
        stats.picture_prepare = started.elapsed();
        stats.layer_count = u32::try_from(frame.tree.layers.len()).unwrap_or(u32::MAX);
        stats.pictures = u32::try_from(frame.pictures.len()).unwrap_or(u32::MAX);
        stats.textures = u32::try_from(device.textures.len()).unwrap_or(u32::MAX);
        self.frame = Some(frame);
        self.reuse_pictures = true;
        #[cfg(target_os = "ios")]
        {
            self.frame_number = self.frame_number.wrapping_add(1);
        }
        self.prepared_buffers = stats.render.buffers_created;
        self.prepared_mesh_bytes = stats.render.mesh_upload_bytes;
        self.stats = stats;
        self.fonts.sweep();
        Ok(SubmitInfo {
            stats,
            platform_views,
        })
    }

    /// Installs layer values and picture replacements over a matching checkpoint.
    ///
    /// Returns `None` when the base is stale or the changes require full picture
    /// preparation. The caller must then submit the full snapshot.
    pub fn submit_layer_delta(
        &mut self,
        tree: &Arc<LayerTree>,
        scale: f32,
        delta: &engine::scene::SceneDelta,
    ) -> Result<Option<SubmitInfo>, Status> {
        use engine::scene::SceneChanges;
        let started = Instant::now();
        let updates = match &delta.changes {
            SceneChanges::LayerValues => &[][..],
            SceneChanges::Pictures(updates) => updates,
            SceneChanges::Topology => return Ok(None),
        };
        let device = self.device.as_ref().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let Some(frame) = self.frame.as_mut().filter(|frame| {
            self.reuse_pictures
                && frame.tree.epoch == delta.base_epoch
                && frame.scale == scale
                && frame.tree.layers.len() == tree.layers.len()
                && frame.tree.tracks.is_empty()
                && tree.tracks.is_empty()
                && frame.gpu_view_requests.is_empty()
                && !frame.pending_images
                && delta.removed_or_changed == delta.inserted_or_changed
        }) else {
            return Ok(None);
        };
        if delta.inserted_or_changed.iter().any(|id| {
            let index = id.0 as usize;
            match (frame.tree.layers.get(index), tree.layers.get(index)) {
                (Some(before), Some(after)) => {
                    before.parent != after.parent
                        || before.next != after.next
                        || before.first_child != after.first_child
                        || !patch_preserves_picture_scale(&before.kind, &after.kind)
                }
                _ => true,
            }
        }) {
            return Ok(None);
        }
        let mut stats = SubmitStats::default();
        let mut prepared = HashMap::with_capacity(updates.len());
        let pool = self.parallel_prepare.then(prepare::pool).flatten();
        for update in updates {
            let Some(picture) = tree.pictures.get(update.index) else {
                return Ok(None);
            };
            let Some(before) = frame.pictures.get(&update.id) else {
                return Ok(None);
            };
            if picture.id != update.id || !picture.images().is_empty() || !before.images.is_empty()
            {
                return Ok(None);
            }
            let compiled = compiler::prepare(
                picture,
                before.scale,
                pool,
                device.gpu.analytic_canvas_curves(),
                &mut |_| textures::Load::Failed,
            )?
            .install(&device.gpu, device.generation, true)?;
            if let Some(mesh) = compiled
                .mesh_buffer
                .get()
                .and_then(batch::MeshBuffer::resident)
            {
                stats.render.buffers_created += 2;
                stats.render.mesh_upload_bytes += mesh.vertices.size() + mesh.indices.size();
            }
            prepared.insert(picture.id, Arc::new(compiled));
        }
        let complex_damage = frame.complex_damage
            || prepared
                .values()
                .any(|picture| picture.has_shader_effect || picture.reads_root);
        let damage = damage::changed_layers(
            frame,
            tree,
            |id| {
                prepared
                    .get(&id)
                    .or_else(|| frame.pictures.get(&id))
                    .map(Arc::as_ref)
            },
            complex_damage,
            delta,
        );
        frame.complex_damage = complex_damage;
        frame.has_glyphs |= prepared.values().any(|picture| picture.has_glyphs);
        frame.pictures.extend(prepared);
        frame.shader_uniforms.clone_from(&tree.shader_uniforms);
        frame.tree = Arc::clone(tree);
        frame.damage = damage;
        frame.previous_revision = frame.revision;
        frame.revision = frame.revision.wrapping_add(1);
        frame.scene_revision = frame.revision;
        let pictures = u32::try_from(frame.pictures.len()).unwrap_or(u32::MAX);
        stats.damage = damage;
        stats.pictures = pictures;
        stats.picture_rebuilt = u32::try_from(updates.len()).unwrap_or(u32::MAX);
        stats.picture_reused = pictures.saturating_sub(stats.picture_rebuilt);
        stats.layer_count = u32::try_from(tree.layers.len()).unwrap_or(u32::MAX);
        stats.textures = u32::try_from(device.textures.len()).unwrap_or(u32::MAX);
        stats.picture_prepare = started.elapsed();
        self.stats = stats;
        self.prepared_buffers = stats.render.buffers_created;
        self.prepared_mesh_bytes = stats.render.mesh_upload_bytes;
        #[cfg(target_os = "ios")]
        {
            self.frame_number = self.frame_number.wrapping_add(1);
        }
        Ok(Some(SubmitInfo {
            stats: self.stats,
            platform_views: Arc::clone(&frame.views),
        }))
    }

    fn encode_and_submit(
        &mut self,
        target: &wgpu::TextureView,
        format: impl Into<compositor::OutputFormat>,
        pixels: PixelSize,
    ) -> Result<(), Status> {
        let mut capture = self.begin_gpu_capture();
        let producers = self.render_gpu_views(capture.as_mut())?;
        let work = FrameWork {
            capture,
            producers,
            ..FrameWork::default()
        };
        self.submit_frame(work, |compositor, gpu, scene, capture| {
            surface::check_size(gpu, pixels)?;
            compositor.begin_frame();
            let command = compositor.encode(
                gpu,
                scene,
                target,
                format,
                [pixels.width, pixels.height],
                capture.map(|capture| (capture, None)),
            )?;
            Ok((Vec::new(), command))
        })
    }

    fn prepare_surface_frame(
        &mut self,
        format: compositor::OutputFormat,
        pixels: PixelSize,
    ) -> Result<PreparedSurfaceFrame, Status> {
        let mut capture = self.begin_gpu_capture();
        let producers = self.render_gpu_views(capture.as_mut())?;
        let started = Instant::now();
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        let frame = self.frame.as_ref().ok_or(Status::NotReady)?;
        surface::check_size(&device.gpu, pixels)?;
        device.compositor.begin_frame();
        let encode_started = Instant::now();
        let _span = tracing::debug_span!(target: "impasto::render", "encode").entered();
        let output = device
            .gpu
            .hot_validation(|| {
                device.compositor.prepare_surface(
                    &device.gpu,
                    frame.scene(&self.fonts, &device.textures, None),
                    format,
                    [pixels.width, pixels.height],
                    capture.as_mut(),
                )
            })
            .inspect_err(|_| device.compositor.discard_frame())?;
        let encode_elapsed = encode_started.elapsed();
        self.stats.encode = encode_elapsed;
        Ok(PreparedSurfaceFrame {
            output,
            work: FrameWork {
                capture,
                producers,
                encode_elapsed,
                prepare_elapsed: started.elapsed(),
            },
        })
    }

    fn submit_surface_frame(
        &mut self,
        target: &wgpu::TextureView,
        prepared: PreparedSurfaceFrame,
    ) -> Result<(), Status> {
        self.submit_frame(prepared.work, |compositor, _, _, _| {
            Ok(prepared.output.finish(target, compositor))
        })
    }

    fn submit_frame(
        &mut self,
        work: FrameWork,
        encode: impl FnOnce(
            &mut Compositor,
            &Gpu,
            Scene<'_>,
            Option<&mut Capture>,
        ) -> Result<EncodedFrame, Status>,
    ) -> Result<(), Status> {
        let FrameWork {
            mut capture,
            producers,
            prepare_elapsed,
            encode_elapsed,
        } = work;
        let started = Instant::now();
        let _compose = tracing::debug_span!(target: "impasto::render", "compose").entered();
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        let gpu = &device.gpu;
        let compositor = &mut device.compositor;
        let (atlas, encode, flush, submit_done, queue_submit) = gpu
            .hot_validation(|| {
                let stage = Instant::now();
                let (passes, command) = {
                    let _span = tracing::debug_span!(target: "impasto::render", "encode").entered();
                    let frame = self.frame.as_ref().ok_or(Status::NotReady)?;
                    let scene = frame.scene(&self.fonts, &device.textures, None);
                    let (passes, mut tail) = encode(compositor, gpu, scene, capture.as_mut())?;
                    if gpu.backend != RasterBackend::Metal
                        && let Some(capture) = &capture
                    {
                        capture.resolve(&mut tail);
                    }
                    (passes, tail.finish())
                };
                let encode = encode_elapsed + stage.elapsed();
                let stage = Instant::now();
                let (atlas, prelude) = {
                    let _span =
                        tracing::debug_span!(target: "impasto::render", "flush_arena").entered();
                    compositor.flush(gpu)?
                };
                let flush = stage.elapsed();
                let stage = Instant::now();
                {
                    let _span =
                        tracing::debug_span!(target: "impasto::render", "queue_submit").entered();
                    gpu.submit(
                        producers
                            .into_iter()
                            .chain(prelude)
                            .chain(passes)
                            .chain([command]),
                    );
                    compositor.submitted(gpu);
                }
                let done = Instant::now();
                Ok((atlas, encode, flush, done, done.duration_since(stage)))
            })
            .inspect_err(|_| compositor.discard_frame())?;
        if let (Some(profiler), Some(capture)) = (&mut device.profiler, capture) {
            self.gpu_sample = profiler.submitted(gpu, capture);
        }
        self.stats.atlas = atlas;
        self.stats.encode = encode;
        self.stats.flush = flush;
        self.stats.queue_submit = queue_submit;
        self.stats.submit_done = Some(submit_done);
        let render = compositor.render_stats();
        self.finish_render_stats(render);
        self.stats.compose = prepare_elapsed + started.elapsed();
        Ok(())
    }
}

impl Frame {
    fn scene<'a>(
        &'a self,
        fonts: &'a FontLibrary,
        textures: &'a TextureStore,
        slice: Option<&'a HashSet<LayerId>>,
    ) -> Scene<'a> {
        Scene {
            tree: &self.tree,
            pictures: &self.pictures,
            fonts,
            textures,
            shader_uniforms: &self.shader_uniforms,
            scale: self.scale,
            slice,
            damage: self.damage,
            has_glyphs: self.has_glyphs,
            revision: self.revision,
            scene_revision: self.scene_revision,
            previous_revision: self.previous_revision,
        }
    }
}

fn patch_preserves_picture_scale(before: &LayerKind, after: &LayerKind) -> bool {
    match (before, after) {
        (LayerKind::Transform { .. }, LayerKind::Transform { .. }) => {
            let [a, b, c, d, ..] = layer_transform(before);
            let [x, y, z, w, ..] = layer_transform(after);
            [a, b, c, d] == [x, y, z, w]
        }
        (LayerKind::Picture { picture: a }, LayerKind::Picture { picture: b }) => a == b,
        (LayerKind::Paint(_), LayerKind::Paint(_))
        | (LayerKind::Opacity { .. }, LayerKind::Opacity { .. })
        | (
            LayerKind::Clip { .. } | LayerKind::CornerClip { .. },
            LayerKind::Clip { .. } | LayerKind::CornerClip { .. },
        )
        | (LayerKind::Offset { .. }, LayerKind::Offset { .. }) => true,
        _ => false,
    }
}

fn picture_scales(
    tree: &LayerTree,
    order: &[LayerId],
    scale: f32,
) -> Result<HashMap<PictureId, f32>, Status> {
    let mut transforms: Vec<Affine> = vec![IDENTITY; tree.layers.len()];
    let mut pictures = HashMap::<PictureId, f32>::new();
    for id in order {
        let index = id.0 as usize;
        let layer = &tree.layers[index];
        let parent = layer
            .parent
            .map_or(IDENTITY, |parent| transforms[parent.0 as usize]);
        let transform = compose(parent, layer_transform(&layer.kind));
        if !transform.iter().all(|value| value.is_finite()) {
            return Err(Status::BadScene);
        }
        transforms[index] = transform;
        if let LayerKind::Picture { picture } = layer.kind {
            let [a, b, c, d, ..] = transform;
            // Half-octave buckets keep scrolling and small transform updates
            // from rebuilding tessellation while bounding pixel error.
            let magnitude = (scale * a.hypot(b).max(c.hypot(d))).clamp(0.25, 128.0);
            let bucket = (magnitude.log2() * 2.0).ceil() * 0.5;
            let scale = bucket.exp2();
            pictures
                .entry(picture)
                .and_modify(|previous| *previous = previous.max(scale))
                .or_insert(scale);
        }
    }
    Ok(pictures)
}

fn paint_order(tree: &LayerTree) -> Result<Vec<LayerId>, Status> {
    if tree.layers.is_empty() {
        return Ok(Vec::new());
    }
    let mut pending = vec![(tree.root, None, 0)];
    let mut seen = vec![false; tree.layers.len()];
    let mut ordered = Vec::with_capacity(tree.layers.len());
    while let Some((id, parent, depth)) = pending.pop() {
        let index = id.0 as usize;
        let layer = tree.layers.get(index).ok_or(Status::BadScene)?;
        if seen[index] || layer.parent != parent || depth > 256 {
            return Err(Status::BadScene);
        }
        seen[index] = true;
        ordered.push(id);
        if let Some(next) = layer.next {
            pending.push((next, parent, depth));
        }
        if let Some(child) = layer.first_child {
            if matches!(
                layer.kind,
                LayerKind::Picture { .. }
                    | LayerKind::PlatformView { .. }
                    | LayerKind::Fill { .. }
                    | LayerKind::Paint(_)
            ) {
                return Err(Status::BadScene);
            }
            pending.push((child, Some(id), depth + 1));
        }
    }
    if ordered.len() != tree.layers.len() {
        return Err(Status::BadScene);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests;
