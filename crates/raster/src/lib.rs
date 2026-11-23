//! wgpu raster and Parley typography for [`engine::scene::LayerTree`].

mod arena;
mod atlas;
mod batch;
mod color;
mod compiler;
mod compositor;
mod fonts;
mod gpu;
mod pipelines;
mod shaper;
mod status;
mod text_index;
mod textures;

pub use fonts::{FontLibrary, ShapeStats};
pub use shaper::ParleyShaper;
pub use status::{RasterBackend, RasterOptions, Status};

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
pub use compositor::RenderStats;
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
use compositor::{Compositor, Scene};
use gpu::Gpu;
use textures::TextureStore;

static NEXT_DEVICE: AtomicU64 = AtomicU64::new(1);

/// Counts from scene submission and the most recent encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmitStats {
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layer_count: u32,
    pub picture_prepare: Duration,
    pub compose: Duration,
    pub pictures: u32,
    pub textures: u32,
    pub atlas: AtlasStats,
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
    device: Option<DeviceState>,
    frame: Option<Frame>,
    stats: SubmitStats,
    reuse_pictures: bool,
    prepared_buffers: u32,
    #[cfg(target_os = "ios")]
    frame_number: u64,
}

struct DeviceState {
    gpu: Gpu,
    generation: u64,
    compositor: Compositor,
    textures: TextureStore,
}

struct Frame {
    tree: LayerTree,
    scale: f32,
    pictures: HashMap<PictureId, Arc<CompiledPicture>>,
    textures: TextureStore,
    views: Arc<[u64]>,
    #[cfg(target_os = "ios")]
    slices: Vec<HashSet<LayerId>>,
}

impl Rasterizer {
    /// Defers adapter selection until the first surface is attached.
    pub fn create(options: RasterOptions, fonts: FontLibrary) -> Result<Self, Status> {
        Ok(Self {
            options,
            fonts,
            device: None,
            frame: None,
            stats: SubmitStats::default(),
            reuse_pictures: false,
            prepared_buffers: 0,
            #[cfg(target_os = "ios")]
            frame_number: 0,
        })
    }

    pub fn headless(options: RasterOptions, fonts: FontLibrary) -> Result<Self, Status> {
        let mut raster = Self::create(options, fonts)?;
        let mut last = Status::NotReady;
        for backend in raster.backends() {
            match Gpu::request(Gpu::instance(backend), backend, options, None)
                .and_then(Self::new_device)
            {
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

    fn new_device(gpu: Gpu) -> Result<DeviceState, Status> {
        let generation = NEXT_DEVICE.fetch_add(1, Ordering::Relaxed);
        let compositor = gpu.validation(|| Compositor::new(&gpu))?;
        let mut textures = TextureStore::default();
        textures.begin_frame(generation);
        Ok(DeviceState {
            gpu,
            generation,
            compositor,
            textures,
        })
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
        }
    }

    /// Clears the working image cache when the host changes decode scale.
    /// The installed frame keeps using its previous textures until resubmission.
    pub fn invalidate_textures(&mut self) {
        if let Some(device) = self.device.as_mut() {
            device.textures.clear();
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
        mut resolve: impl FnMut(&str) -> Option<ImagePixels<'pixels>>,
    ) -> Result<SubmitInfo, Status> {
        if !point_scale.is_finite() || point_scale <= 0.0 {
            return Err(Status::BadScene);
        }
        let started = Instant::now();
        let order = paint_order(tree)?;
        let scales = picture_scales(tree, &order, point_scale)?;
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        if device.gpu.is_lost() {
            return Err(Status::DeviceLost);
        }
        let mut textures = device.textures.clone();
        textures.begin_frame(device.generation);
        let previous = self.frame.as_ref().filter(|_| self.reuse_pictures);
        let mut stats = SubmitStats::default();
        let mut pictures = HashMap::with_capacity(tree.pictures.len());
        for picture in &tree.pictures {
            let compile_scale = scales.get(&picture.id).copied().unwrap_or(point_scale);
            let cached = previous
                .and_then(|frame| {
                    frame
                        .pictures
                        .get(&picture.id)
                        .map(|cached| (frame, cached))
                })
                .filter(|(frame, cached)| {
                    cached.device == device.generation
                        && cached.generation == picture.generation
                        && cached.scale == compile_scale
                        && cached.origin == picture.origin
                        && cached
                            .images
                            .iter()
                            .all(|(source, _)| textures.same_geometry(source, &frame.textures))
                        && cached.waiting.iter().all(|source| {
                            textures.get(source).is_none() && resolve(source).is_none()
                        })
                });
            let compiled = if let Some((_, cached)) = cached {
                for (source, _) in &cached.images {
                    textures.touch(source);
                }
                stats.picture_reused = stats.picture_reused.saturating_add(1);
                Arc::clone(cached)
            } else {
                let mut context = CompileContext {
                    gpu: &device.gpu,
                    device_generation: device.generation,
                    textures: &mut textures,
                    resolve: &mut resolve,
                    point_scale: compile_scale,
                };
                let result = compiler::compile(picture, &mut context)?;
                stats.picture_rebuilt = stats.picture_rebuilt.saturating_add(1);
                if result.mesh_buffer.get().is_some() {
                    stats.render.buffers_created += 2;
                }
                Arc::new(result)
            };
            if pictures.insert(picture.id, compiled).is_some() {
                return Err(Status::BadScene);
            }
        }
        let mut views = Vec::new();
        #[cfg(target_os = "ios")]
        let mut slices = vec![HashSet::new()];
        for id in order {
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
                LayerKind::Picture { .. } => {
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
        textures.evict_unused();
        let platform_views: Arc<[u64]> = views.into();
        stats.picture_prepare = started.elapsed();
        stats.layer_count = u32::try_from(tree.layers.len()).unwrap_or(u32::MAX);
        stats.pictures = u32::try_from(pictures.len()).unwrap_or(u32::MAX);
        stats.textures = u32::try_from(textures.len()).unwrap_or(u32::MAX);
        device.textures = textures.clone();
        self.frame = Some(Frame {
            tree: tree.clone(),
            scale: point_scale,
            pictures,
            textures,
            views: Arc::clone(&platform_views),
            #[cfg(target_os = "ios")]
            slices,
        });
        self.reuse_pictures = true;
        #[cfg(target_os = "ios")]
        {
            self.frame_number = self.frame_number.wrapping_add(1);
        }
        self.prepared_buffers = stats.render.buffers_created;
        self.stats = stats;
        self.fonts.sweep();
        Ok(SubmitInfo {
            stats,
            platform_views,
        })
    }

    fn encode_and_submit(
        &mut self,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        pixels: PixelSize,
    ) -> Result<(), Status> {
        let device = self.device.as_mut().ok_or(Status::NotReady)?;
        let frame = self.frame.as_ref().ok_or(Status::NotReady)?;
        surface::check_size(&device.gpu, pixels)?;
        let started = Instant::now();
        let gpu = &device.gpu;
        let compositor = &mut device.compositor;
        compositor.begin_frame();
        self.stats.atlas = gpu.hot_validation(|| {
            let command = compositor.encode(
                gpu,
                frame.scene(&self.fonts, None),
                target,
                format,
                [pixels.width, pixels.height],
            )?;
            let atlas = compositor.flush(gpu);
            gpu.queue.submit([command]);
            Ok(atlas)
        })?;
        self.stats.render = compositor.render_stats();
        self.stats.render.buffers_created += std::mem::take(&mut self.prepared_buffers);
        self.stats.compose = started.elapsed();
        Ok(())
    }
}

impl Frame {
    fn scene<'a>(
        &'a self,
        fonts: &'a FontLibrary,
        slice: Option<&'a HashSet<LayerId>>,
    ) -> Scene<'a> {
        Scene {
            tree: &self.tree,
            pictures: &self.pictures,
            fonts,
            textures: &self.textures,
            scale: self.scale,
            slice,
        }
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
                LayerKind::Picture { .. } | LayerKind::PlatformView { .. }
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
