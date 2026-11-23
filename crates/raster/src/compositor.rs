//! Ordered scene replay, clipping, and offscreen composition.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use engine::scene::{LayerId, LayerKind, LayerTree, PictureId};

use crate::atlas::{Atlas, AtlasStats};
use crate::batch::{self, GlyphInstance, ImageInstance, Mesh, RectInstance};
use crate::compiler::{
    self, Affine, Clip, ClipShape, CompiledPicture, Draw, Effect, Group, IDENTITY, Scope,
};
use crate::gpu::Gpu;
use crate::pipelines::{self, ParamsUniform, PipelineCache, Target, ViewUniform};
use crate::{FontLibrary, Status, textures::TextureStore};

const INSTANCES: usize = 64;
const POOL_LIMIT: usize = 16;
const POOL_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct Compositor {
    atlas: Atlas,
    pipelines: PipelineCache,
    pool: Vec<Image>,
    white: wgpu::TextureView,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    render_stats: RenderStats,
}

/// CPU-side encoding counters from the last rendered frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub draws: u32,
    pub passes: u32,
    pub pooled_texture_bytes: u64,
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
    format: wgpu::TextureFormat,
    samples: u32,
}

impl Image {
    fn new(gpu: &Gpu, size: [u32; 2], format: wgpu::TextureFormat, samples: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impasto offscreen"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: if samples == 1 {
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
            format,
            samples,
        }
    }

    fn bytes(&self) -> u64 {
        let channels = if self.format == wgpu::TextureFormat::R8Unorm {
            1
        } else {
            4
        };
        u64::from(self.size[0]) * u64::from(self.size[1]) * u64::from(self.samples) * channels
    }
}

impl Compositor {
    pub fn new(gpu: &Gpu) -> Self {
        let white = Image::new(gpu, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1);
        gpu.queue.write_texture(
            white.texture.as_image_copy(),
            &[255; 4],
            Default::default(),
            extent([1, 1]),
        );
        Self {
            atlas: Atlas::new(gpu, white.view.clone()),
            pipelines: PipelineCache::new(gpu),
            pool: Vec::new(),
            white: white.view,
            linear: pipelines::linear_sampler(gpu),
            nearest: pipelines::nearest_sampler(gpu),
            render_stats: RenderStats::default(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.atlas.begin_frame();
        self.render_stats = RenderStats::default();
    }

    pub fn flush(&mut self, gpu: &Gpu) -> AtlasStats {
        self.atlas.flush(gpu);
        self.atlas.take_stats()
    }

    pub fn render_stats(&self) -> RenderStats {
        RenderStats {
            pooled_texture_bytes: self.pool.iter().map(Image::bytes).sum(),
            ..self.render_stats
        }
    }

    pub fn trim(&mut self) {
        self.atlas.trim();
        self.pool.clear();
    }

    pub fn encode(
        &mut self,
        gpu: &Gpu,
        scene: Scene<'_>,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: [u32; 2],
    ) -> Result<wgpu::CommandBuffer, Status> {
        let command = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("impasto frame"),
            });
        let mut frame = Frame {
            gpu,
            scene,
            compositor: self,
            command,
            pending: None,
            size,
        };
        let main = frame.take(size, wgpu::TextureFormat::Rgba8UnormSrgb);
        frame.clear(&main.view);
        if !frame.scene.tree.layers.is_empty() {
            frame.layers(
                Some(frame.scene.tree.root),
                &main,
                IDENTITY,
                &Clip::default(),
            )?;
        }
        frame.full(
            &main,
            None,
            None,
            output,
            format,
            [0.0; 4],
            FullPass::Copy,
            None,
        )?;
        frame.release(main);
        Ok(frame.command.finish())
    }
}

struct Frame<'a> {
    gpu: &'a Gpu,
    scene: Scene<'a>,
    compositor: &'a mut Compositor,
    command: wgpu::CommandEncoder,
    pending: Option<DrawPass>,
    size: [u32; 2],
}

struct DrawPass {
    target: wgpu::TextureView,
    calls: Vec<DrawCall>,
}

struct DrawCall {
    pipeline: wgpu::RenderPipeline,
    bindings: wgpu::BindGroup,
    scissor: Option<[u32; 4]>,
    instances: u32,
}

#[derive(Clone, Copy)]
enum FullPass {
    Copy,
    Over,
    Blend,
    Backdrop,
    Blur,
}

#[derive(Default)]
struct Coverage {
    scissor: Option<[u32; 4]>,
    mask: Option<Image>,
    empty: bool,
}

impl Frame<'_> {
    fn record(
        &mut self,
        target: &wgpu::TextureView,
        pipeline: wgpu::RenderPipeline,
        bindings: wgpu::BindGroup,
        scissor: Option<[u32; 4]>,
        instances: u32,
    ) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pass| &pass.target != target)
        {
            self.flush_draws();
        }
        let pending = self.pending.get_or_insert_with(|| DrawPass {
            target: target.clone(),
            calls: Vec::new(),
        });
        self.compositor.render_stats.draws = self.compositor.render_stats.draws.saturating_add(1);
        pending.calls.push(DrawCall {
            pipeline,
            bindings,
            scissor,
            instances,
        });
    }

    fn flush_draws(&mut self) {
        let Some(batch) = self.pending.take() else {
            return;
        };
        self.compositor.render_stats.passes = self.compositor.render_stats.passes.saturating_add(1);
        let mut pass = render(&mut self.command, &batch.target, wgpu::LoadOp::Load);
        let mut scissor = None;
        for call in &batch.calls {
            if call.scissor != scissor {
                set_scissor(
                    &mut pass,
                    Some(call.scissor.unwrap_or([0, 0, self.size[0], self.size[1]])),
                );
                scissor = call.scissor;
            }
            pass.set_pipeline(&call.pipeline);
            pass.set_bind_group(0, &call.bindings, &[]);
            pass.draw(0..6, 0..call.instances);
        }
    }

    fn clear(&mut self, view: &wgpu::TextureView) {
        self.flush_draws();
        self.compositor.render_stats.passes = self.compositor.render_stats.passes.saturating_add(1);
        drop(render(
            &mut self.command,
            view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        ));
    }

    fn take(&mut self, size: [u32; 2], format: wgpu::TextureFormat) -> Image {
        self.take_sampled(size, format, 1)
    }

    fn take_sampled(&mut self, size: [u32; 2], format: wgpu::TextureFormat, samples: u32) -> Image {
        self.compositor
            .pool
            .iter()
            .position(|image| {
                image.size == size && image.format == format && image.samples == samples
            })
            .map_or_else(
                || Image::new(self.gpu, size, format, samples),
                |at| self.compositor.pool.swap_remove(at),
            )
    }

    fn release(&mut self, image: Image) {
        let pool = &mut self.compositor.pool;
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
                    self.scopes(&picture, &picture.scopes, 0, target, local, clip)?;
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
                    };
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
        picture: &CompiledPicture,
        mut scopes: &[Scope],
        depth: usize,
        target: &Image,
        origin: Affine,
        parent_clip: &Clip,
    ) -> Result<(), Status> {
        while let Some((scope, rest)) = scopes.split_first() {
            let state = &scope.state;
            if let Some(id) = state.groups.get(depth) {
                let length = scopes
                    .iter()
                    .take_while(|scope| scope.state.groups.get(depth) == Some(id))
                    .count();
                let (group_scopes, remaining) = scopes.split_at(length);
                let group = *picture.groups.get(*id).ok_or(Status::BadScene)?;
                let transform = compiler::compose(origin, state.transform);
                let clip = merge_clip(parent_clip, &state.clip, origin);
                self.group(target, transform, &clip, group, |frame, target| {
                    frame.scopes(
                        picture,
                        group_scopes,
                        depth + 1,
                        target,
                        origin,
                        parent_clip,
                    )
                })?;
                scopes = remaining;
                continue;
            }
            scopes = rest;
            if scope.draws.is_empty() {
                continue;
            }
            let transform = compiler::compose(origin, state.transform);
            let clip = merge_clip(parent_clip, &state.clip, origin);
            let coverage = self.coverage(&clip)?;
            if !coverage.empty {
                self.draws(target, transform, &coverage, &scope.draws)?;
            }
            if let Some(mask) = coverage.mask {
                self.release(mask);
            }
        }
        Ok(())
    }

    fn coverage(&mut self, clip: &Clip) -> Result<Coverage, Status> {
        let mut coverage = Coverage::default();
        if let Some(bounds) = clip.scissor {
            coverage.scissor = scissor(bounds, self.scene.scale, self.size);
            if coverage.scissor.is_none() {
                coverage.empty = true;
                return Ok(coverage);
            }
        }
        for shape in &clip.shapes {
            let next = self.take(self.size, wgpu::TextureFormat::R8Unorm);
            self.clear(&next.view);
            let transform = shape.transform;
            if let Some(mesh) = shape.mesh.as_deref() {
                self.mesh(&next, transform, &coverage, mesh, true)?;
            } else {
                let mut rect = compiler::solid_rect(shape.rect, [1.0; 4]);
                rect.radius = shape.radius;
                self.rects(&next, transform, &coverage, &[rect], &[])?;
            }
            if let Some(previous) = coverage.mask.replace(next) {
                self.release(previous);
            }
        }
        Ok(coverage)
    }

    fn draws(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        draws: &[Draw],
    ) -> Result<(), Status> {
        let mut solids = Vec::new();
        for draw in draws {
            if let Draw::Rects(rects, None) = draw {
                solids.extend(rects.iter().cloned());
                continue;
            }
            if !solids.is_empty() {
                self.rects(target, transform, clip, &solids, &[])?;
                solids.clear();
            }
            match draw {
                Draw::Rects(instances, stops) => {
                    self.rects(
                        target,
                        transform,
                        clip,
                        instances,
                        stops.as_deref().unwrap_or(&[]),
                    )?;
                }
                Draw::Fill(color) => {
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
                Draw::Glyphs(glyphs) => self.glyphs(target, transform, clip, glyphs)?,
                Draw::Images { source, instances } => {
                    if let Some(image) = self.scene.textures.get(source) {
                        self.images(
                            target,
                            transform,
                            clip,
                            instances,
                            image.view(),
                            image.premultiplied_srgb,
                        )?;
                    }
                }
                Draw::Mesh(mesh) => self.mesh(target, transform, clip, mesh, false)?,
            }
        }
        if !solids.is_empty() {
            self.rects(target, transform, clip, &solids, &[])?;
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
        for chunk in rects.chunks(INSTANCES) {
            let words = chunk.iter().flat_map(|rect| {
                let mut rect = rect.clone();
                rect.clip_mask[2] = f32::from(clip.mask.is_some());
                rect.pad[0] = 1.0;
                rect.words()
            });
            let data = self.instances(words, RectInstance::WORDS)?;
            let view = self.transform(transform)?;
            let stops = self.stops(stops)?;
            let mask = self.mask(clip);
            let bind = self.bind(
                &self.compositor.pipelines.layouts.ui,
                &[
                    view.as_entire_binding(),
                    stops.as_entire_binding(),
                    data.as_entire_binding(),
                    wgpu::BindingResource::TextureView(mask),
                    wgpu::BindingResource::Sampler(&self.compositor.linear),
                ],
            );
            let pipelines = self
                .compositor
                .pipelines
                .get(self.gpu, single(target.format))?;
            let pipeline = pipelines.rect.clone();
            self.record(
                &target.view,
                pipeline,
                bind,
                clip.scissor,
                draw_count(chunk.len())?,
            );
        }
        Ok(())
    }

    fn mesh(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        mesh: &Mesh,
        mask: bool,
    ) -> Result<(), Status> {
        if mesh.indices.is_empty() {
            return Ok(());
        }
        let mut paint =
            compiler::solid_rect([0.0; 4], if mask { [1.0; 4] } else { mesh.brush.solid() });
        let stops = if let (false, batch::Brush::Gradient(gradient)) = (mask, &mesh.brush) {
            set_rect_gradient(&mut paint, gradient);
            gradient.stops.as_slice()
        } else {
            &[]
        };
        let vertices = pipelines::buffer_from(
            self.gpu,
            "canvas path",
            bytemuck::cast_slice(&mesh.vertices),
            wgpu::BufferUsages::VERTEX,
        )?;
        let indices = pipelines::buffer_from(
            self.gpu,
            "canvas indices",
            bytemuck::cast_slice(&mesh.indices),
            wgpu::BufferUsages::INDEX,
        )?;
        let data = self.instances(paint.words(), RectInstance::WORDS)?;
        let stops = self.stops(stops)?;
        let bounds = mesh_bounds(mesh, transform);
        let Some(mut area) = scissor(bounds, self.scene.scale, self.size) else {
            return Ok(());
        };
        if let Some(clip_area) = clip.scissor {
            let Some(intersection) = intersect_pixels(area, clip_area) else {
                return Ok(());
            };
            area = intersection;
        }
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let features = format.guaranteed_format_features(self.gpu.device.features());
        let samples = if features.flags.contains(
            wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        ) {
            4
        } else {
            1
        };
        let [x, y, width, height] = area;
        let size = [width, height];
        let scale = self.scene.scale;
        let origin = [x as f32 / scale, y as f32 / scale];
        let view = self.transform_in(transform, size, origin)?;
        let bind = self.bind(
            &self.compositor.pipelines.layouts.ui,
            &[
                view.as_entire_binding(),
                stops.as_entire_binding(),
                data.as_entire_binding(),
                wgpu::BindingResource::TextureView(&self.compositor.white),
                wgpu::BindingResource::Sampler(&self.compositor.linear),
            ],
        );
        let resolved = self.take(size, format);
        let multisample = (samples > 1).then(|| self.take_sampled(size, format, samples));
        self.flush_draws();
        let pipeline = &self
            .compositor
            .pipelines
            .get(self.gpu, Target { format, samples })?
            .mesh;
        {
            let view = multisample
                .as_ref()
                .map_or(&resolved.view, |image| &image.view);
            let resolve_target = multisample.as_ref().map(|_| &resolved.view);
            let mut pass = self.command.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas path coverage"),
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..draw_count(mesh.indices.len())?, 0, 0..1);
        }
        self.images(
            target,
            IDENTITY,
            clip,
            &[ImageInstance {
                rect: [
                    origin[0],
                    origin[1],
                    width as f32 / scale,
                    height as f32 / scale,
                ],
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [1.0; 4],
                flags: 0,
                clip_rect: [0.0; 4],
                clip_mask: [0.0; 4],
            }],
            &resolved.view,
            false,
        )?;
        // The next use of the coverage attachment can overwrite it only
        // after the queued composite above has been encoded.
        self.flush_draws();
        self.release(resolved);
        if let Some(multisample) = multisample {
            self.release(multisample);
        }
        Ok(())
    }

    fn images(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        images: &[ImageInstance],
        image: &wgpu::TextureView,
        premultiplied_srgb: bool,
    ) -> Result<(), Status> {
        let nearest = |image: &ImageInstance| image.flags & batch::IMAGE_NEAREST != 0;
        let chunks = images
            .chunk_by(|a, b| nearest(a) == nearest(b))
            .flat_map(|group| group.chunks(INSTANCES));
        for chunk in chunks {
            let sampler = if nearest(&chunk[0]) {
                &self.compositor.nearest
            } else {
                &self.compositor.linear
            };
            let words = chunk.iter().flat_map(|instance| {
                let mut instance = instance.clone();
                instance.clip_mask[2] = f32::from(clip.mask.is_some());
                if premultiplied_srgb {
                    instance.flags |= batch::IMAGE_PREMULTIPLIED_SRGB;
                }
                instance.words()
            });
            let data = self.instances(words, ImageInstance::WORDS)?;
            let view = self.transform(transform)?;
            let bind = self.bind(
                &self.compositor.pipelines.layouts.image,
                &[
                    view.as_entire_binding(),
                    data.as_entire_binding(),
                    wgpu::BindingResource::TextureView(image),
                    wgpu::BindingResource::Sampler(sampler),
                    wgpu::BindingResource::TextureView(self.mask(clip)),
                    wgpu::BindingResource::Sampler(&self.compositor.linear),
                ],
            );
            let pipelines = self
                .compositor
                .pipelines
                .get(self.gpu, single(target.format))?;
            let pipeline = pipelines.image.clone();
            self.record(
                &target.view,
                pipeline,
                bind,
                clip.scissor,
                draw_count(chunk.len())?,
            );
        }
        Ok(())
    }

    fn glyphs(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        glyphs: &[compiler::GlyphDraw],
    ) -> Result<(), Status> {
        let mut batch = Vec::new();
        let mut key = (false, usize::MAX, None);
        let scale = self.scene.scale;
        let [a, b, c, d, ..] = transform;
        let pixel_scale = scale * a.hypot(b).max(c.hypot(d)).max(0.001);
        let aligned = b.abs() < 0.0001 && c.abs() < 0.0001 && a > 0.0 && (a - d).abs() < 0.0001;
        for glyph in glyphs {
            let (rect, uv, color, page, decoration) = if let Some(rect) = glyph.decoration {
                (rect, [0.0; 4], false, usize::MAX, true)
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
                (
                    rect,
                    self.compositor.atlas.uv(placement),
                    placement.color,
                    placement.rect.page,
                    false,
                )
            };
            let next = (color, page, glyph.gradient.clone());
            if !batch.is_empty() && (key != next || batch.len() == INSTANCES) {
                self.glyph_batch(target, transform, clip, &batch, &key)?;
                batch.clear();
            }
            key = next;
            let mut instance = GlyphInstance {
                rect,
                uv,
                color: glyph.color,
                flags: 0,
                gradient: [0.0; 4],
                gradient_a: [0.0; 4],
                span: [0.0; 4],
                clip_rect: [0.0; 4],
                clip_mask: [0.0, 0.0, f32::from(clip.mask.is_some()), 0.0],
                pad: [glyph.color[3], 0.0, 0.0, 0.0],
            };
            if color {
                instance.flags |= batch::GLYPH_COLOR;
            }
            if decoration {
                instance.flags |= batch::GLYPH_DECORATION;
            }
            if let Some(gradient) = glyph.gradient.as_deref() {
                instance.flags |= batch::GLYPH_GRADIENT;
                instance.gradient = [
                    gradient.kind as f32,
                    0.0,
                    (gradient.stops.len() / 2) as f32,
                    0.0,
                ];
                instance.gradient_a = gradient.a;
                instance.span[..2].copy_from_slice(&gradient.radial_radii);
            }
            batch.push(instance);
        }
        if !batch.is_empty() {
            self.glyph_batch(target, transform, clip, &batch, &key)?;
        }
        Ok(())
    }

    fn glyph_batch(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        glyphs: &[GlyphInstance],
        key: &(bool, usize, Option<Arc<batch::PackedGradient>>),
    ) -> Result<(), Status> {
        let (colored, page, gradient) = key;
        let atlas = &self.compositor.atlas;
        let (mask_page, color_page) = if *colored {
            (&self.compositor.white, atlas.color_view(*page))
        } else {
            (atlas.mask_view(*page), &self.compositor.white)
        };
        let data = self.instances(
            glyphs.iter().flat_map(GlyphInstance::words),
            GlyphInstance::WORDS,
        )?;
        let view = self.transform(transform)?;
        let stops = self.stops(gradient.as_deref().map_or(&[], |g| g.stops.as_slice()))?;
        let bind = self.bind(
            &self.compositor.pipelines.layouts.text,
            &[
                view.as_entire_binding(),
                stops.as_entire_binding(),
                data.as_entire_binding(),
                wgpu::BindingResource::TextureView(mask_page),
                wgpu::BindingResource::TextureView(color_page),
                wgpu::BindingResource::Sampler(&self.compositor.linear),
                wgpu::BindingResource::TextureView(self.mask(clip)),
                wgpu::BindingResource::Sampler(&self.compositor.linear),
            ],
        );
        let pipelines = self
            .compositor
            .pipelines
            .get(self.gpu, single(target.format))?;
        let pipeline = pipelines.text.clone();
        self.record(
            &target.view,
            pipeline,
            bind,
            clip.scissor,
            draw_count(glyphs.len())?,
        );
        Ok(())
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
        let scale = self.scene.scale
            * transform[0]
                .hypot(transform[1])
                .max(transform[2].hypot(transform[3]));
        let radius = group.sigma * scale;
        let mut group_clip = clip.clone();
        if let Some(bounds) = group.bounds {
            add_rect_clip(&mut group_clip, bounds, 0.0, transform);
        }
        if group.effect == Effect::BackdropBlur {
            if radius.is_finite() && radius > 0.0 {
                let coverage = self.coverage(&group_clip)?;
                if !coverage.empty {
                    let snapshot = self.snapshot(target);
                    let blurred = self.blur(&snapshot, radius)?;
                    self.full(
                        &blurred,
                        Some(&snapshot),
                        coverage.mask.as_ref(),
                        &target.view,
                        target.format,
                        [0.0; 4],
                        FullPass::Backdrop,
                        coverage.scissor,
                    )?;
                    self.release(blurred);
                    self.release(snapshot);
                }
                if let Some(mask) = coverage.mask {
                    self.release(mask);
                }
            }
            return children(self, target);
        }

        let source = self.take(target.size, target.format);
        self.clear(&source.view);
        children(self, &source)?;
        let blurred = if group.effect == Effect::Blur && radius.is_finite() && radius > 0.0 {
            Some(self.blur(&source, radius)?)
        } else {
            None
        };
        let mode = match group.effect {
            Effect::Blend(mode) => batch::blend_mode(mode),
            Effect::Canvas(mode) => batch::canvas_composite(mode),
            _ => 0,
        };
        let snapshot = (mode != 0).then(|| self.snapshot(target));
        let coverage = if blurred.is_some() {
            self.coverage(&group_clip)?
        } else {
            Coverage::default()
        };
        if !coverage.empty {
            self.full(
                blurred.as_ref().unwrap_or(&source),
                snapshot.as_ref(),
                coverage.mask.as_ref(),
                &target.view,
                target.format,
                [0.0, 0.0, group.opacity, mode as f32],
                if mode == 0 {
                    FullPass::Over
                } else {
                    FullPass::Blend
                },
                coverage.scissor,
            )?;
        }
        if let Some(mask) = coverage.mask {
            self.release(mask);
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

    fn snapshot(&mut self, target: &Image) -> Image {
        self.flush_draws();
        let snapshot = self.take(target.size, target.format);
        self.command.copy_texture_to_texture(
            target.texture.as_image_copy(),
            snapshot.texture.as_image_copy(),
            extent(target.size),
        );
        snapshot
    }

    fn blur(&mut self, source: &Image, radius: f32) -> Result<Image, Status> {
        let mut levels = Vec::new();
        let mut size = source.size;
        let mut remaining = radius;
        while remaining > 2.0 && levels.len() < 4 && size[0].min(size[1]) > 1 {
            size = [size[0].div_ceil(2), size[1].div_ceil(2)];
            let down = self.take(size, source.format);
            let from = levels.last().unwrap_or(source);
            self.blur_pass(from, &down, 1.0)?;
            levels.push(down);
            remaining *= 0.5;
        }
        let mut filtered = self.take(size, source.format);
        self.blur_pass(
            levels.last().unwrap_or(source),
            &filtered,
            remaining.clamp(0.1, 4.0),
        )?;
        while let Some(level) = levels.pop() {
            let size = levels.last().map_or(source.size, |image| image.size);
            let next = self.take(size, source.format);
            self.blur_pass(&filtered, &next, 0.75)?;
            self.release(level);
            self.release(filtered);
            filtered = next;
        }
        Ok(filtered)
    }

    fn blur_pass(&mut self, source: &Image, to: &Image, radius: f32) -> Result<(), Status> {
        self.full(
            source,
            None,
            None,
            &to.view,
            to.format,
            [
                1.0 / source.size[0] as f32,
                1.0 / source.size[1] as f32,
                radius,
                0.0,
            ],
            FullPass::Blur,
            None,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a blit specifies its attachments, uniforms and draw state"
    )]
    fn full(
        &mut self,
        source: &Image,
        backdrop: Option<&Image>,
        mask: Option<&Image>,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        params: [f32; 4],
        kind: FullPass,
        scissor: Option<[u32; 4]>,
    ) -> Result<(), Status> {
        self.flush_draws();
        let view = self.uniform(&ViewUniform {
            transform: [
                [2.0, 0.0, 0.0, 0.0],
                [0.0, -2.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
            scale: 1.0,
            pad: [0.0; 3],
        })?;
        let params = self.uniform(&ParamsUniform { values: params })?;
        let sampler = wgpu::BindingResource::Sampler(&self.compositor.linear);
        let input = self.bind(
            &self.compositor.pipelines.layouts.blur,
            &[
                view.as_entire_binding(),
                params.as_entire_binding(),
                wgpu::BindingResource::TextureView(&source.view),
                sampler.clone(),
            ],
        );
        let read_target = matches!(kind, FullPass::Over | FullPass::Blend | FullPass::Backdrop);
        let extra = read_target.then(|| {
            self.bind(
                &self.compositor.pipelines.layouts.composite,
                &[
                    wgpu::BindingResource::TextureView(
                        backdrop.map_or(&self.compositor.white, |image| &image.view),
                    ),
                    sampler,
                    wgpu::BindingResource::TextureView(
                        mask.map_or(&self.compositor.white, |image| &image.view),
                    ),
                ],
            )
        });
        let pipelines = self.compositor.pipelines.get(self.gpu, single(format))?;
        let (pipeline, load) = match kind {
            FullPass::Copy if !format.is_srgb() => (
                &pipelines.copy_srgb,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            ),
            FullPass::Copy => (
                &pipelines.copy,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            ),
            FullPass::Blur => (
                &pipelines.blur,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            ),
            FullPass::Over => (&pipelines.over, wgpu::LoadOp::Load),
            FullPass::Blend => (&pipelines.composite, wgpu::LoadOp::Load),
            FullPass::Backdrop => (&pipelines.backdrop, wgpu::LoadOp::Load),
        };
        self.compositor.render_stats.passes = self.compositor.render_stats.passes.saturating_add(1);
        self.compositor.render_stats.draws = self.compositor.render_stats.draws.saturating_add(1);
        let mut pass = render(&mut self.command, target, load);
        set_scissor(&mut pass, scissor);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &input, &[]);
        if let Some(extra) = &extra {
            pass.set_bind_group(1, extra, &[]);
        }
        pass.draw(0..6, 0..1);
        Ok(())
    }

    fn uniform<T: bytemuck::Pod>(&self, value: &T) -> Result<wgpu::Buffer, Status> {
        pipelines::buffer_from(
            self.gpu,
            "impasto uniform",
            bytemuck::bytes_of(value),
            wgpu::BufferUsages::UNIFORM,
        )
    }

    fn transform(&self, transform: Affine) -> Result<wgpu::Buffer, Status> {
        self.transform_in(transform, self.size, [0.0; 2])
    }

    fn transform_in(
        &self,
        t: Affine,
        size: [u32; 2],
        origin: [f32; 2],
    ) -> Result<wgpu::Buffer, Status> {
        let sx = 2.0 * self.scene.scale / size[0] as f32;
        let sy = -2.0 * self.scene.scale / size[1] as f32;
        self.uniform(&ViewUniform {
            transform: [
                [t[0] * sx, t[1] * sy, 0.0, 0.0],
                [t[2] * sx, t[3] * sy, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [
                    (t[4] - origin[0]) * sx - 1.0,
                    (t[5] - origin[1]) * sy + 1.0,
                    0.0,
                    1.0,
                ],
            ],
            scale: self.scene.scale * t[0].hypot(t[1]).min(t[2].hypot(t[3])).max(0.001),
            pad: [self.size[0] as f32, self.size[1] as f32, 0.0],
        })
    }

    fn stops(&self, stops: &[[f32; 4]]) -> Result<wgpu::Buffer, Status> {
        let mut data = [[0.0; 4]; pipelines::MAX_STOPS * 2];
        for (dest, source) in data.iter_mut().zip(stops) {
            *dest = *source;
        }
        pipelines::buffer_from(
            self.gpu,
            "gradient stops",
            bytemuck::cast_slice(&data),
            wgpu::BufferUsages::UNIFORM,
        )
    }

    fn instances(
        &self,
        words: impl IntoIterator<Item = f32>,
        stride: usize,
    ) -> Result<wgpu::Buffer, Status> {
        let mut data: Vec<f32> = words.into_iter().collect();
        data.resize(INSTANCES * stride, 0.0);
        pipelines::buffer_from(
            self.gpu,
            "draw instances",
            bytemuck::cast_slice(&data),
            wgpu::BufferUsages::UNIFORM,
        )
    }

    fn mask<'a>(&'a self, coverage: &'a Coverage) -> &'a wgpu::TextureView {
        coverage
            .mask
            .as_ref()
            .map_or(&self.compositor.white, |image| &image.view)
    }

    fn bind(
        &self,
        layout: &wgpu::BindGroupLayout,
        resources: &[wgpu::BindingResource<'_>],
    ) -> wgpu::BindGroup {
        let entries: Vec<_> = (0..)
            .zip(resources.iter().cloned())
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
            .collect();
        self.gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("impasto draw"),
                layout,
                entries: &entries,
            })
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

fn render<'a>(
    command: &'a mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'a> {
    command.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("impasto ordered pass"),
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

fn mesh_bounds(mesh: &Mesh, transform: Affine) -> [f32; 4] {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for &[x, y, _] in &mesh.vertices {
        let (x, y) = compiler::apply(transform, x, y);
        min = [min[0].min(x), min[1].min(y)];
        max = [max[0].max(x), max[1].max(y)];
    }
    [min[0], min[1], max[0] - min[0], max[1] - min[1]]
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
    } else if radius <= 0.0 && transform[1].abs() < 0.0001 && transform[2].abs() < 0.0001 {
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
    } else {
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
        add_rect_clip(&mut result, rect, 0.0, transform);
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
