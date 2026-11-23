//! Ordered scene replay, clipping, and offscreen composition.

use crate::profiler::{Capture, GpuPassKind};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use engine::scene::{LayerId, LayerKind, LayerTree, PictureId};

use bytemuck::Zeroable;

use crate::arena::{Address, DynamicGroup, Extra, FrameArena, PaintPage, VertexSlice};
use crate::atlas::{Atlas, AtlasBinding, AtlasStats};
use crate::batch::{self, GlyphVertex, ImageInstance, Mesh, PaintExtra, RectInstance, RectVertex};
use crate::compiler::{
    self, Affine, Clip, ClipShape, CompiledPicture, Draw, Effect, Group, IDENTITY, PictureBounds,
    Scope,
};
use crate::compute_blur::{BlurParams, ComputeBlur};
use crate::gpu::Gpu;
use crate::pipelines::{
    self, BlendKey, ParamsUniform, PipelineCache, PipelineKind, Target, ViewUniform,
};
use crate::{FontLibrary, Status, textures::TextureStore};

const POOL_LIMIT: usize = 16;
const POOL_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct Compositor {
    atlas: Atlas,
    pipelines: PipelineCache,
    pool: Vec<Image>,
    white: wgpu::TextureView,
    white_mask_group: wgpu::BindGroup,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    trilinear: wgpu::Sampler,
    render_stats: RenderStats,
    arena: FrameArena,
    composites: HashMap<(wgpu::TextureView, wgpu::TextureView), wgpu::BindGroup>,
    masks: HashMap<MaskKey, Image>,
    offscreen_live_bytes: u64,
    frame_active: bool,
    trim_pending: bool,
    specialize_paint: bool,
    compute_blur: Option<ComputeBlur>,
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
    transform: [u32; 6],
    mesh: Option<(wgpu::Buffer, std::ops::Range<u32>)>,
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
}

pub(crate) struct Scene<'a> {
    pub tree: &'a LayerTree,
    pub pictures: &'a HashMap<PictureId, Arc<CompiledPicture>>,
    pub fonts: &'a FontLibrary,
    pub textures: &'a TextureStore,
    pub scale: f32,
    pub slice: Option<&'a HashSet<LayerId>>,
}

#[derive(Clone)]
struct Image {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    logical: [u32; 2],
    origin: [u32; 2],
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
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto offscreen"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
            logical: size,
            origin: [0; 2],
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
            size,
            logical: size,
            origin: [0; 2],
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

impl Compositor {
    pub fn new(gpu: &Gpu) -> Result<Self, Status> {
        let white = Image::new(gpu, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1);
        gpu.queue.write_texture(
            white.texture.as_image_copy(),
            &[255; 4],
            Default::default(),
            extent([1, 1]),
        );
        let mut pipelines = PipelineCache::new(gpu);
        for kind in [PipelineKind::Rect, PipelineKind::Text, PipelineKind::Image] {
            pipelines.get(
                gpu,
                single(wgpu::TextureFormat::Rgba8UnormSrgb),
                kind,
                BlendKey::SrcOver,
            )?;
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
            pipelines,
            pool: Vec::new(),
            white: white.view,
            white_mask_group,
            linear,
            nearest: pipelines::nearest_sampler(gpu),
            trilinear: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("impasto image trilinear"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
            render_stats: RenderStats::default(),
            arena: FrameArena::new(gpu),
            composites: HashMap::new(),
            masks: HashMap::new(),
            offscreen_live_bytes: 0,
            frame_active: false,
            trim_pending: false,
            specialize_paint: std::env::var_os("IMPASTO_SPECIALIZE_PAINT").is_some(),
            compute_blur: ComputeBlur::new(gpu),
        })
    }

    pub fn begin_frame(&mut self) {
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
    }

    pub fn flush(&mut self, gpu: &Gpu) -> AtlasStats {
        self.atlas.flush(gpu);
        self.arena.flush(gpu);
        self.recycle_masks();
        let pooled = |view: &wgpu::TextureView| {
            *view == self.white || self.pool.iter().any(|image| image.view == *view)
        };
        self.composites
            .retain(|(backdrop, mask), _| pooled(backdrop) && pooled(mask));
        self.render_stats.pooled_texture_bytes = self.pool.iter().map(Image::bytes).sum();
        self.render_stats.buffers_created += self.arena.buffers_created;
        self.render_stats.bind_groups_created += self.arena.bind_groups_created;
        [
            self.render_stats.vertex_bytes,
            self.render_stats.uniform_bytes,
            self.render_stats.stop_bytes,
            self.render_stats.extra_bytes,
        ] = self.arena.bytes_uploaded();
        self.render_stats.gradient_lut_bytes = self.arena.lut.upload_bytes;
        self.render_stats.gradient_lut_hits = self.arena.lut.hits;
        let atlas_stats = self.atlas.take_stats();
        self.render_stats.atlas_upload_bytes = atlas_stats.upload_bytes;
        self.render_stats.copy_pixels += atlas_stats.copy_pixels;
        self.frame_active = false;
        if self.trim_pending {
            self.trim_resources();
        }
        atlas_stats
    }

    pub fn render_stats(&self) -> RenderStats {
        self.render_stats
    }

    pub fn trim(&mut self) {
        if self.frame_active {
            // Earlier UIKit slices can still reference data waiting for flush.
            self.trim_pending = true;
        } else {
            self.trim_resources();
        }
    }

    fn trim_resources(&mut self) {
        self.trim_pending = false;
        self.atlas.trim();
        self.pool.clear();
        self.composites.clear();
        self.arena.trim();
        self.masks.clear();
    }

    fn recycle_masks(&mut self) {
        for (_, image) in self.masks.drain() {
            release_image(&mut self.pool, image);
        }
    }

    pub fn encode(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        capture: Option<(&mut Capture, Option<usize>)>,
    ) -> Result<wgpu::CommandEncoder, Status> {
        let command = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("impasto frame"),
            });
        let direct = format.is_srgb() && !scene_reads_root(&scene, Some(scene.tree.root));
        let first = self.render_stats.passes == 0;
        self.render_stats.direct_present = direct && (first || self.render_stats.direct_present);
        let output = Image::output(output, format, size);
        let mut frame = Frame {
            gpu,
            scene,
            compositor: self,
            command,
            pending: None,
            clears: HashMap::new(),
            root: output.view.clone(),
            main: output.view.clone(),
            capture,
            size,
        };
        let main = if direct {
            output.clone()
        } else {
            frame.take(size, wgpu::TextureFormat::Rgba8UnormSrgb)
        };
        frame.main = main.view.clone();
        frame.clear(&main.view);
        if !frame.scene.tree.layers.is_empty() {
            frame.layers(
                Some(frame.scene.tree.root),
                &main,
                IDENTITY,
                &Clip::default(),
            )?;
        }
        frame.finish(&main.view);
        if !direct {
            frame.clear(&output.view);
            frame.full(
                &main,
                None,
                &Coverage::default(),
                &output,
                output.area(),
                FullPass::Copy,
            )?;
            frame.release(main);
        }
        Ok(frame.command)
    }
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
            LayerKind::Clip { .. } | LayerKind::Offset { .. } | LayerKind::Transform { .. }
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

impl ImageSource<'_> {
    fn material(self, frame: &mut Frame<'_>, nearest: bool) -> wgpu::BindGroup {
        let (view, cache) = match self {
            Self::External(image) => (image.view(), image.material.as_ref()),
            Self::Pooled(image) => (&image.view, image.material.as_ref()),
        };
        let compositor = &mut frame.compositor;
        let sampler = if nearest {
            &compositor.nearest
        } else if matches!(self, Self::External(image) if image.mipmapped) {
            &compositor.trilinear
        } else {
            &compositor.linear
        };
        let (group, created) = cache.get(
            frame.gpu,
            &compositor.pipelines.layouts.material,
            view,
            sampler,
            nearest,
        );
        compositor.render_stats.bind_groups_created += u32::from(created);
        group
    }

    fn premultiplied_srgb(self) -> bool {
        matches!(self, Self::External(image) if image.premultiplied_srgb)
    }
}

struct PictureReplay<'a> {
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
    clears: HashMap<wgpu::TextureView, wgpu::Color>,
    root: wgpu::TextureView,
    main: wgpu::TextureView,
    capture: Option<(&'a mut Capture, Option<usize>)>,
    size: [u32; 2],
}

struct DrawPass {
    format: wgpu::TextureFormat,
    target: wgpu::TextureView,
    size: [u32; 2],
    calls: Vec<DrawCall>,
}

struct DrawCall {
    pipeline: wgpu::RenderPipeline,
    globals: DynamicGroup,
    material: wgpu::BindGroup,
    mask: Option<wgpu::BindGroup>,
    vertices: VertexSlice,
    scissor: Option<[u32; 4]>,
    instances: u32,
}

#[derive(Clone, Copy)]
enum FullPass {
    Copy,
    Over { blend: BlendKey, opacity: f32 },
    Multiply { opacity: f32 },
    Backdrop,
    BlurDown { radius: f32, backdrop: bool },
    BlurUp { radius: f32, backdrop: bool },
    MaskIntersect,
}

#[derive(Clone, Default)]
struct Coverage {
    scissor: Option<[u32; 4]>,
    mask: Option<Image>,
    empty: bool,
    boxes: [[f32; 4]; 4],
    radii: [f32; 4],
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
            target_info: [
                target.origin[0] as f32,
                target.origin[1] as f32,
                target.size[0] as f32,
                target.size[1] as f32,
            ],
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
    ) -> Result<(), Status> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pass| pass.target != target.view)
        {
            self.flush_draws();
        }
        let pending = self.pending.get_or_insert_with(|| DrawPass {
            format: target.format,
            target: target.view.clone(),
            size: target.logical,
            calls: Vec::new(),
        });
        let limit = FrameArena::vertex_limit::<T>(self.gpu);
        if limit == 0 {
            return Err(Status::OutOfMemory);
        }
        for chunk in vertices.chunks(limit) {
            self.compositor.render_stats.draws += 1;
            pending.calls.push(DrawCall {
                pipeline: pipeline.clone(),
                globals: globals.clone(),
                material: material.clone(),
                mask: mask.clone(),
                vertices: self.compositor.arena.vertices(self.gpu, chunk)?,
                scissor: scissor.map(|area| target.local(area)),
                instances: draw_count(chunk.len())?,
            });
        }
        Ok(())
    }

    fn flush_draws(&mut self) {
        let Some(batch) = self.pending.take() else {
            return;
        };
        self.count_pass(&batch.target);
        let load = self.load(&batch.target);
        let kind = self.pass_kind(&batch.target, batch.format);
        let stamps = pass_timestamps(
            &mut self.capture,
            kind,
            u32::try_from(batch.calls.len()).unwrap_or(u32::MAX),
        );
        let mut pass = render(&mut self.command, &batch.target, load, stamps);
        let mut scissor = None;
        set_scissor(&mut pass, Some([0, 0, batch.size[0], batch.size[1]]));
        for call in &batch.calls {
            if call.scissor != scissor {
                set_scissor(
                    &mut pass,
                    Some(call.scissor.unwrap_or([0, 0, batch.size[0], batch.size[1]])),
                );
                scissor = call.scissor;
            }
            pass.set_pipeline(&call.pipeline);
            pass.set_bind_group(0, &call.globals.group, call.globals.offsets());
            pass.set_bind_group(1, &call.material, &[]);
            if let Some(mask) = &call.mask {
                pass.set_bind_group(2, mask, &[]);
            }
            pass.set_vertex_buffer(0, call.vertices.slice());
            pass.draw(0..6, 0..call.instances);
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
        self.flush_draws();
        if let Some(color) = self.clears.remove(view) {
            self.count_pass(view);
            let kind = if *view == self.main {
                GpuPassKind::Main
            } else {
                GpuPassKind::OffscreenEffect
            };
            let stamps = pass_timestamps(&mut self.capture, kind, 0);
            drop(render(
                &mut self.command,
                view,
                wgpu::LoadOp::Clear(color),
                stamps,
            ));
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
        let [x, y, w, h] = area;
        let size = [w, h].map(|side| side.next_multiple_of(64).min(self.gpu.caps.max_texture_2d));
        let compositor = &mut self.compositor;
        let index = compositor.pool.iter().position(|image| {
            image.size == size
                && image.format == format
                && image.samples == samples
                && image.usage == usage
        });
        let mut image = if let Some(index) = index {
            compositor.pool.swap_remove(index)
        } else {
            let image = Image::with_usage(self.gpu, size, format, samples, usage);
            compositor.render_stats.offscreen_alloc_bytes += image.bytes();
            image
        };
        compositor.offscreen_live_bytes += image.bytes();
        compositor.render_stats.offscreen_peak_bytes = compositor
            .render_stats
            .offscreen_peak_bytes
            .max(compositor.offscreen_live_bytes);
        image.logical = [w, h];
        image.origin = [x, y];
        image
    }

    fn release(&mut self, image: Image) {
        self.compositor.offscreen_live_bytes -= image.bytes();
        release_image(&mut self.compositor.pool, image);
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
            let children = layer.first_child;
            let child = match layer.kind {
                LayerKind::PlatformView { .. } => PictureBounds::Empty,
                LayerKind::Picture { picture } => {
                    let picture = self.scene.pictures.get(&picture).ok_or(Status::BadScene)?;
                    let origin = compiler::compose(
                        transform,
                        translation(picture.origin.0, picture.origin.1),
                    );
                    picture.bounds_in(origin, self.scene.scale).0
                }
                LayerKind::Offset { .. } | LayerKind::Transform { .. } => self.layer_bounds(
                    children,
                    compiler::compose(transform, compiler::layer_transform(&layer.kind)),
                )?,
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
                _ => self.layer_bounds(children, transform)?,
            };
            bounds = bounds.union(child);
        }
        Ok(bounds)
    }

    fn layers(
        &mut self,
        mut current: Option<LayerId>,
        target: &Image,
        transform: Affine,
        clip: &Clip,
    ) -> Result<(), Status> {
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
            let children = layer.first_child;
            match layer.kind {
                LayerKind::PlatformView { .. } => {}
                LayerKind::Picture { picture } => {
                    let picture =
                        Arc::clone(self.scene.pictures.get(&picture).ok_or(Status::BadScene)?);
                    let local = compiler::compose(
                        transform,
                        translation(picture.origin.0, picture.origin.1),
                    );
                    let replay = PictureReplay {
                        picture: &picture,
                        origin: local,
                        group_bounds: if picture.groups.is_empty() {
                            None
                        } else {
                            picture.bounds_in(local, self.scene.scale).1
                        },
                    };
                    self.scopes(&replay, &picture.scopes, 0, target, clip)?;
                }
                LayerKind::Offset { .. } | LayerKind::Transform { .. } => {
                    let local = compiler::layer_transform(&layer.kind);
                    self.layers(children, target, compiler::compose(transform, local), clip)?;
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
                LayerKind::Opacity { alpha, .. } => {
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
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
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
                    self.group(target, transform, clip, group, |frame, target| {
                        frame.layers(children, target, transform, clip)
                    })?;
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

    fn scopes(
        &mut self,
        replay: &PictureReplay<'_>,
        mut scopes: &[Scope],
        depth: usize,
        target: &Image,
        parent_clip: &Clip,
    ) -> Result<(), Status> {
        let origin = replay.origin;
        while let Some((scope, rest)) = scopes.split_first() {
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
                if !self.fold_opacity(
                    group,
                    group_scopes,
                    depth + 1,
                    target,
                    origin,
                    parent_clip,
                )? {
                    self.group(target, transform, &clip, group, |frame, target| {
                        frame.scopes(replay, group_scopes, depth + 1, target, parent_clip)
                    })?;
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
            let coverage = self.coverage(target, &clip)?;
            if !coverage.empty {
                self.draws(target, transform, &coverage, &scope.draws)?;
            }
        }
        Ok(())
    }

    fn coverage(&mut self, target: &Image, clip: &Clip) -> Result<Coverage, Status> {
        let mut coverage = Coverage::default();
        let mut area = target.area();
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
                coverage.radii[coverage.count] = shape.radius * a * scale;
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
                    transform: shape.transform.map(f32::to_bits),
                    mesh: shape.mesh.as_deref().and_then(|mesh| {
                        Some((mesh.buffer.get()?.indices.clone(), mesh.indices.clone()))
                    }),
                })
                .collect(),
        };
        if let Some(image) = self.compositor.masks.get(&key) {
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
                self.rects(&next, shape.transform, &unclipped, &[rect], &[])?;
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
            if self.compositor.masks.len() >= POOL_LIMIT
                && let Some(old) = self.compositor.masks.keys().next().cloned()
                && let Some(evicted) = self.compositor.masks.remove(&old)
            {
                self.release(evicted);
            }
            self.compositor.masks.insert(key, image.clone());
            coverage.mask = Some(image);
        }
        Ok(coverage)
    }

    fn area(&self, bounds: PictureBounds, target: &Image) -> Option<[u32; 4]> {
        match bounds {
            PictureBounds::Empty => None,
            PictureBounds::Unbounded => Some(target.area()),
            PictureBounds::Rect(bounds) => scissor(bounds, self.scene.scale, self.size)
                .and_then(|area| intersect_pixels(area, target.area())),
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
        let mut nonempty = scopes.iter().filter(|scope| !scope.draws.is_empty());
        let Some(scope) = nonempty.next() else {
            return Ok(true);
        };
        if nonempty.next().is_some() || scope.state.groups.len() != expected_groups {
            return Ok(false);
        }
        let transform = compiler::compose(origin, scope.state.transform);
        let clip = merge_clip(parent_clip, &scope.state.clip, origin);
        match scope.draws.as_slice() {
            [Draw::Rects(rects, stops, BlendKey::SrcOver)]
                if rects.len() == 1 && rects[0].kind == batch::RECT_FILL =>
            {
                let mut rect = rects[0].clone();
                rect.gradient_b[3] *= group.opacity;
                let coverage = self.coverage(target, &clip)?;
                if !coverage.empty {
                    self.rects(
                        target,
                        transform,
                        &coverage,
                        &[rect],
                        stops.as_deref().unwrap_or_default(),
                    )?;
                }
            }
            [
                Draw::Images {
                    source,
                    instances,
                    blend: BlendKey::SrcOver,
                },
            ] if instances.len() == 1 => {
                if let Some(image) = self.scene.textures.get(source) {
                    let mut instance = instances[0].clone();
                    instance.color = instance.color.map(|channel| channel * group.opacity);
                    let coverage = self.coverage(target, &clip)?;
                    if !coverage.empty {
                        self.images(
                            target,
                            transform,
                            &coverage,
                            &[instance],
                            ImageSource::External(image),
                            BlendKey::SrcOver,
                        )?;
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn draws(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        draws: &[Draw],
    ) -> Result<(), Status> {
        let mut draws = draws.iter().peekable();
        while let Some(draw) = draws.next() {
            match draw {
                Draw::Rects(instances, stops, blend) => {
                    let mut first = Some((instances, stops));
                    let batches = std::iter::from_fn(|| {
                        if let Some(first) = first.take() {
                            return Some(first);
                        }
                        let Draw::Rects(rects, stops, next_blend) = *draws.peek()? else {
                            return None;
                        };
                        if next_blend != blend {
                            return None;
                        }
                        draws.next();
                        Some((rects, stops))
                    });
                    self.rect_batches(
                        target,
                        transform,
                        clip,
                        *blend,
                        batches.map(|(rects, stops)| {
                            (rects.as_slice(), stops.as_deref().unwrap_or_default())
                        }),
                    )?;
                }
                Draw::Fill(color) => {
                    if clip.mask.is_none()
                        && clip.scissor.is_none()
                        && clip.count == 0
                        && self.pending.is_none()
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
                    self.rects(target, IDENTITY, clip, &[rect], &[])?;
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
    ) -> Result<(), Status> {
        self.rect_batches(target, transform, clip, BlendKey::SrcOver, [(rects, stops)])
    }

    fn rect_batches<'a>(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        blend: BlendKey,
        batches: impl IntoIterator<Item = (&'a [RectInstance], &'a [[f32; 4]])>,
    ) -> Result<(), Status> {
        let view = self.transform(target, transform, clip)?;
        let mut batch = Vec::new();
        let mut active = None;
        for (rects, stops) in batches {
            let ramp = self.compositor.arena.lut.layer(self.gpu, stops);
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
        }
        if let Some(page) = active {
            self.rect_batch(target, clip, view, page, &batch, blend)?;
        }
        Ok(())
    }

    fn rect_batch(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: Address,
        page: PaintPage,
        vertices: &[RectVertex],
        blend: BlendKey,
    ) -> Result<(), Status> {
        if vertices.is_empty() {
            return Ok(());
        }
        let globals = self.globals(view, Extra::Paint(page));
        let mask = self.mask_group(clip);
        let kind = if self.compositor.specialize_paint
            && clip.mask.is_none()
            && clip.count == 0
            && vertices
                .iter()
                .all(|rect| rect.shape[2] == 0.0 && rect.paint == u32::MAX)
        {
            PipelineKind::RectSolid
        } else {
            PipelineKind::Rect
        };
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), kind, blend)?;
        self.record(
            target,
            pipeline,
            globals,
            mask,
            None,
            vertices,
            clip.scissor,
        )
    }

    fn meshes(
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
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let samples = if self.gpu.caps.msaa4_srgb { 4 } else { 1 };
        let resolved = self.take_area(area, format, 1);
        let multisample = (samples > 1).then(|| self.take_area(area, format, samples));
        let view = self.transform(&resolved, transform, &Coverage::default())?;
        let mask_group = self.mask_group(&Coverage::default());
        let mut calls = Vec::with_capacity(meshes.len());
        for mesh in meshes {
            if mesh.indices.is_empty() {
                continue;
            }
            let mut paint = compiler::solid_rect([0.0; 4], mesh.brush.solid());
            let stops = if let batch::Brush::Gradient(gradient) = &mesh.brush {
                set_rect_gradient(&mut paint, gradient);
                gradient.stops.as_slice()
            } else {
                &[]
            };
            let ramp = self.compositor.arena.lut.layer(self.gpu, stops);
            let stops = self.stops(stops)?;
            let (page, index) =
                self.paint(stops, paint.extra(FrameArena::stop_index(stops), ramp))?;
            let data = self
                .compositor
                .arena
                .vertices(self.gpu, &[paint.vertex(index)])?;
            let globals = self.globals(view, Extra::Paint(page));
            calls.push((mesh, data, globals));
        }
        self.flush_draws();
        self.count_pass(&resolved.view);
        self.compositor.render_stats.draws += draw_count(calls.len())?;
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            Target { format, samples },
            PipelineKind::Mesh,
            BlendKey::SrcOver,
        )?;
        {
            let view = multisample
                .as_ref()
                .map_or(&resolved.view, |image| &image.view);
            let resolve_target = multisample.as_ref().map(|_| &resolved.view);
            let kind = if resolve_target.is_some() {
                GpuPassKind::CanvasMsaa
            } else {
                self.pass_kind(&resolved.view, format)
            };
            let timestamp_writes =
                pass_timestamps(&mut self.capture, kind, draw_count(calls.len())?);
            let mut pass = self.command.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas path batch"),
                timestamp_writes,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: if resolve_target.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            set_scissor(&mut pass, Some([0, 0, area[2], area[3]]));
            pass.set_bind_group(1, &mask_group, &[]);
            let mut last = None;
            for (mesh, vertices, globals) in &calls {
                let buffers = mesh.buffer.get().ok_or(Status::BadScene)?;
                if last != Some(buffers) {
                    pass.set_vertex_buffer(0, buffers.vertices.slice(..));
                    pass.set_index_buffer(buffers.indices.slice(..), wgpu::IndexFormat::Uint32);
                    last = Some(buffers);
                }
                pass.set_bind_group(0, &globals.group, globals.offsets());
                pass.set_vertex_buffer(1, vertices.slice());
                pass.draw_indexed(mesh.indices.clone(), 0, 0..1);
            }
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
        self.flush_draws();
        self.release(resolved);
        if let Some(multisample) = multisample {
            self.release(multisample);
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
        let view = self.transform(target, transform, clip)?;
        let mask = self.mask_group(clip);
        let globals = self.globals(view, Extra::None);
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            PipelineKind::Image,
            blend,
        )?;
        let mut vertices = Vec::new();
        let mut images = images.into_iter().peekable();
        while let Some(first) = images.next() {
            let nearest = first.flags & batch::IMAGE_NEAREST;
            vertices.push(first.vertex(image.premultiplied_srgb()));
            while let Some(next) =
                images.next_if(|next| next.flags & batch::IMAGE_NEAREST == nearest)
            {
                vertices.push(next.vertex(image.premultiplied_srgb()));
            }
            let material = image.material(self, nearest != 0);
            self.record(
                target,
                pipeline.clone(),
                globals.clone(),
                material,
                Some(mask.clone()),
                &vertices,
                clip.scissor,
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
        let view = self.transform(target, transform, clip)?;
        let mut last_gradient: Option<(&batch::PackedGradient, PaintPage, u32)> = None;
        let scale = self.scene.scale;
        let [a, b, c, d, ..] = transform;
        let pixel_scale = scale * a.hypot(b).max(c.hypot(d)).max(0.001);
        let aligned = b.abs() < 0.0001 && c.abs() < 0.0001 && a > 0.0 && (a - d).abs() < 0.0001;
        for glyph in glyphs {
            let (rect, uv, color, binding, layer, decoration) = if let Some(rect) = glyph.decoration
            {
                (rect, [0; 4], false, key, 0, true)
            } else {
                let (x, y) = compiler::apply(transform, glyph.x, glyph.y);
                let (sx, sy) = if aligned {
                    (x * scale, y * scale)
                } else {
                    (0.0, 0.0)
                };
                let Some(placement) = self.compositor.atlas.glyph(
                    self.gpu,
                    self.scene.fonts,
                    glyph,
                    glyph.size * pixel_scale,
                    aligned.then_some([sx, sy]),
                ) else {
                    continue;
                };
                if placement.rect.w == 0 || placement.rect.h == 0 {
                    continue;
                }
                let dx = if aligned {
                    sx.rem_euclid(1.0) / (scale * a)
                } else {
                    0.0
                };
                let dy = if aligned {
                    sy.rem_euclid(1.0) / (scale * d)
                } else {
                    0.0
                };
                let rect = [
                    glyph.x - dx + placement.left as f32 / pixel_scale,
                    glyph.y - dy - placement.top as f32 / pixel_scale,
                    placement.rect.w as f32 / pixel_scale,
                    placement.rect.h as f32 / pixel_scale,
                ];
                let (binding, layer) = self.compositor.atlas.binding(placement);
                (
                    rect,
                    self.compositor
                        .atlas
                        .texels(placement)
                        .ok_or(Status::BadScene)?,
                    placement.color,
                    binding,
                    layer,
                    false,
                )
            };
            let mut flags = if color { batch::GLYPH_COLOR } else { 0 };
            if decoration {
                flags |= batch::GLYPH_DECORATION;
            }
            let (page, index) = if let Some(gradient) = glyph.gradient.as_deref().filter(|_| !color)
            {
                flags |= batch::GLYPH_GRADIENT;
                match last_gradient {
                    Some((last, page, index))
                        if std::ptr::eq(last, gradient) && page.same_bindings(active) =>
                    {
                        (page, index)
                    }
                    _ => {
                        let stops = self.stops(&gradient.stops)?;
                        let ramp = self.compositor.arena.lut.layer(self.gpu, &gradient.stops);
                        let (page, index) = self.paint(
                            stops,
                            Some(PaintExtra {
                                geometry: gradient.a,
                                gradient: [
                                    gradient.kind as f32,
                                    FrameArena::stop_index(stops),
                                    (gradient.stops.len() / 2) as f32,
                                    ramp as f32,
                                ],
                                shadow: [
                                    gradient.radial_radii[0],
                                    gradient.radial_radii[1],
                                    0.0,
                                    0.0,
                                ],
                                stroke: [0.0; 4],
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
            batch.push(GlyphVertex {
                rect,
                uv,
                color: glyph
                    .color
                    .map(|value| half::f16::from_f32(value).to_bits()),
                flags_and_layer: flags | (layer << 8),
                paint: index,
            });
        }
        self.glyph_batch(target, clip, view, active, &batch, key)
    }

    fn glyph_batch(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: Address,
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
        let globals = self.globals(view, Extra::Paint(page));
        let mask = self.mask_group(clip);
        let kind = if self.compositor.specialize_paint
            && clip.mask.is_none()
            && clip.count == 0
            && glyphs.iter().all(|glyph| glyph.flags_and_layer & 255 == 0)
        {
            PipelineKind::TextMask
        } else {
            PipelineKind::Text
        };
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            kind,
            BlendKey::SrcOver,
        )?;
        self.record(
            target,
            pipeline,
            globals,
            material,
            Some(mask),
            glyphs,
            clip.scissor,
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
        if let Some(bounds) = group.bounds {
            add_rect_clip(&mut group_clip, bounds, 0.0, transform);
        }
        if group.effect == Effect::BackdropBlur {
            if valid_blur {
                let coverage = self.coverage(target, &group_clip)?;
                if !coverage.empty {
                    let dst = coverage.scissor.unwrap_or_else(|| target.area());
                    let src = expand_pixels(dst, radius, target.area());
                    let snapshot = self.snapshot(target, src);
                    let blurred = self.blur(&snapshot, radius, true)?;
                    let kind = if coverage.mask.is_none() && coverage.count == 0 {
                        FullPass::Copy
                    } else {
                        FullPass::Backdrop
                    };
                    self.full(&blurred, Some(&snapshot), &coverage, target, dst, kind)?;
                    self.release(blurred);
                    self.release(snapshot);
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
        let source = self.take_area(area, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
        self.clear(&source.view);
        children(self, &source)?;
        self.finish(&source.view);
        let blurred = if filter {
            Some(self.blur(&source, radius, false)?)
        } else {
            None
        };
        let mode = match group.effect {
            Effect::Blend(mode) => batch::blend_mode(mode),
            Effect::Canvas(mode) => batch::canvas_composite(mode),
            _ => 0,
        };
        let blend = BlendKey::from_mode(mode);
        let snapshot = blend.is_none().then(|| self.snapshot(target, area));
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
                    blend.map_or(
                        FullPass::Multiply {
                            opacity: group.opacity,
                        },
                        |blend| FullPass::Over {
                            blend,
                            opacity: group.opacity,
                        },
                    ),
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

    fn snapshot(&mut self, target: &Image, area: [u32; 4]) -> Image {
        self.finish(&target.view);
        let snapshot = self.take_area(area, target.format, 1);
        let [x, y, w, h] = target.local(area);
        let mut from = target.texture.as_image_copy();
        from.origin = wgpu::Origin3d { x, y, z: 0 };
        self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
        self.command.copy_texture_to_texture(
            from,
            snapshot.texture.as_image_copy(),
            extent([w, h]),
        );
        snapshot
    }

    fn blur(&mut self, source: &Image, radius: f32, backdrop: bool) -> Result<Image, Status> {
        let mut levels = Vec::new();
        let mut size = source.logical;
        let mut remaining = radius;
        let origin = source.origin;
        let format = source.format;
        while remaining > 2.0 && levels.len() < 4 && size[0].min(size[1]) > 1 {
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
            remaining *= 0.5;
        }
        let input = levels.last().unwrap_or(source);
        // The apron fits sigma <= 8 at this pyramid level. The render path
        // is retained for larger kernels and for capabilities without storage.
        let mut filtered = if (0.25..=8.0).contains(&remaining)
            && let Some(pipeline) = self.compositor.compute_blur.clone()
        {
            self.compute_blur(&pipeline, input, remaining)?
        } else {
            let image = self.take_area([origin[0], origin[1], size[0], size[1]], format, 1);
            self.blur_pass(
                input,
                &image,
                FullPass::BlurDown {
                    radius: remaining.clamp(0.1, 4.0),
                    backdrop,
                },
            )?;
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
        reason = "sigma is between 0.25 and 8 at the call site"
    )]
    fn compute_blur(
        &mut self,
        kernel: &ComputeBlur,
        source: &Image,
        sigma: f32,
    ) -> Result<Image, Status> {
        let format = wgpu::TextureFormat::Rgba16Float;
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
        let scratch = self.take_area_with_usage(source.area(), format, 1, usage);
        let output = self.take_area_with_usage(source.area(), format, 1, usage);
        let [w, h] = source.logical;
        let params = self.uniform(&BlurParams {
            size: [w, h],
            radius: (sigma * 3.0).ceil() as u32,
            sigma,
        })?;
        for (axis, (input, target)) in
            [(&source.view, &scratch.view), (&scratch.view, &output.view)]
                .into_iter()
                .enumerate()
        {
            let pipeline = kernel.pipeline(axis);
            let material = pipelines::bind_group(
                self.gpu,
                &pipeline.get_bind_group_layout(0),
                &[
                    wgpu::BindingResource::TextureView(input),
                    wgpu::BindingResource::TextureView(target),
                    self.compositor.arena.uniform_binding::<BlurParams>(params),
                ],
            );
            let stamps = pass_timestamps(&mut self.capture, GpuPassKind::ComputeBlur, 0);
            let mut pass = self
                .command
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("impasto compute blur"),
                    timestamp_writes: stamps.map(|writes| wgpu::ComputePassTimestampWrites {
                        query_set: writes.query_set,
                        beginning_of_pass_write_index: writes.beginning_of_pass_write_index,
                        end_of_pass_write_index: writes.end_of_pass_write_index,
                    }),
                });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &material, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            drop(pass);
            self.count_pass(target);
            self.compositor.render_stats.compute_passes += 1;
            self.compositor.render_stats.bind_groups_created += 1;
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
            _ => 0.0,
        };
        let params = self.uniform(&ParamsUniform {
            values: [source.origin[0] as f32, source.origin[1] as f32, value, 0.0],
            uv_bounds: source.uv_bounds(),
        })?;
        let globals = self.globals(view, Extra::Params(params));
        let input = ImageSource::Pooled(source).material(self, false);
        let extra = matches!(
            kind,
            FullPass::Over { .. } | FullPass::Multiply { .. } | FullPass::Backdrop
        )
        .then(|| self.composite_group(backdrop, coverage.mask.as_ref()));
        let load = self.load(&target.view);
        self.count_pass(&target.view);
        self.compositor.render_stats.draws += 1;
        let blend = if let FullPass::Over { blend, .. } = kind {
            blend
        } else {
            BlendKey::SrcOver
        };
        let pipeline_kind = match kind {
            FullPass::Copy if !target.format.is_srgb() => PipelineKind::CopySrgb,
            FullPass::Copy => PipelineKind::Copy,
            FullPass::BlurDown { .. } => PipelineKind::BlurDown,
            FullPass::BlurUp { .. } => PipelineKind::BlurUp,
            FullPass::Over { .. } => PipelineKind::Over,
            FullPass::Multiply { .. } => PipelineKind::Multiply,
            FullPass::Backdrop => PipelineKind::Backdrop,
            FullPass::MaskIntersect => PipelineKind::MaskIntersect,
        };
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), pipeline_kind, blend)?;
        let [x, y, w, h] = target.local(area);
        let kind = match kind {
            FullPass::BlurDown { backdrop, .. } | FullPass::BlurUp { backdrop, .. } => {
                self.compositor.render_stats.blur_pixels += u64::from(w) * u64::from(h);
                if backdrop {
                    GpuPassKind::BackdropBlur
                } else {
                    GpuPassKind::OffscreenEffect
                }
            }
            FullPass::Backdrop => GpuPassKind::BackdropBlur,
            FullPass::MaskIntersect => GpuPassKind::ClipMask,
            FullPass::Copy => {
                self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
                GpuPassKind::Copy
            }
            _ => GpuPassKind::OffscreenEffect,
        };
        let stamps = pass_timestamps(&mut self.capture, kind, 1);
        let mut pass = render(&mut self.command, &target.view, load, stamps);
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &globals.group, globals.offsets());
        pass.set_bind_group(1, &input, &[]);
        if let Some(extra) = &extra {
            pass.set_bind_group(2, extra, &[]);
        }
        pass.draw(0..6, 0..1);
        Ok(())
    }

    fn uniform<T: bytemuck::Pod>(&mut self, value: &T) -> Result<Address, Status> {
        self.compositor.arena.uniform(self.gpu, value)
    }

    fn transform(&mut self, target: &Image, t: Affine, clip: &Coverage) -> Result<Address, Status> {
        let [w, h] = target.size.map(|v| v as f32);
        let sx = 2.0 * self.scene.scale / w;
        let sy = -2.0 * self.scene.scale / h;
        let [x, y] = target.origin.map(|v| v as f32 / self.scene.scale);
        let scale = compiler::primitive_scale(t, self.scene.scale);
        self.uniform(&ViewUniform {
            transform: [
                [t[0] * sx, t[1] * sy, 0.0, 0.0],
                [t[2] * sx, t[3] * sy, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [(t[4] - x) * sx - 1.0, (t[5] - y) * sy + 1.0, 0.0, 1.0],
            ],
            ..clip.view(target, scale)
        })
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

fn release_image(pool: &mut Vec<Image>, image: Image) {
    let bytes = image.bytes();
    if bytes > POOL_BYTES {
        return;
    }
    while !pool.is_empty()
        && (pool.len() >= POOL_LIMIT
            || bytes + pool.iter().map(Image::bytes).sum::<u64>() > POOL_BYTES)
    {
        pool.remove(0);
    }
    pool.push(image);
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

fn add_rect_clip(clip: &mut Clip, rect: [f32; 4], radius: f32, transform: Affine) {
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
    if radius > 0.0 || !aligned {
        clip.shapes.push(ClipShape {
            rect,
            radius,
            transform,
            mesh: None,
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
