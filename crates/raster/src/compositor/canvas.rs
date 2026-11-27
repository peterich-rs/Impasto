//! Canvas Lyon meshes, direct coverage fringes, and on-demand MSAA.

use engine::scene::CanvasComposite;

use super::{
    Coverage, DrawPipeline, Frame, Image, ImageSource, draw_count, intersect_pixels,
    pass_timestamps, set_rect_gradient, single,
};
use crate::Status;
use crate::arena::{Address, Extra, FrameArena, PaintPage};
use crate::batch::{self, GpuMeshBuffer, ImageInstance, Mesh, RectVertex};
use crate::compiler::{self, Affine, IDENTITY, PictureBounds};
use crate::gpu::Gpu;
use crate::pipelines::{BlendKey, PipelineKind, Target};
use crate::profiler::GpuPassKind;

struct CanvasTarget {
    area: [u32; 4],
    resolved: Image,
    multisample: Option<Image>,
    mask_target: bool,
}

impl Frame<'_> {
    pub(super) fn meshes(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        meshes: &[&Mesh],
    ) -> Result<(), Status> {
        if clip.count != 0 || clip.mask.is_some() || target.format == wgpu::TextureFormat::R8Unorm {
            return self.meshes_msaa(target, transform, clip, meshes);
        }
        let mut pending = 0;
        let mut view = None;
        let mut direct: Option<(&batch::Brush, compiler::Tessellation)> = None;
        for (index, &mesh) in meshes.iter().enumerate() {
            let Some(geometry) = mesh
                .fringe
                .as_deref()
                .filter(|_| mesh.composite == CanvasComposite::SourceOver)
                .and_then(|points| crate::fringe::mesh(points, transform, self.scene.scale))
            else {
                continue;
            };
            if pending != index {
                self.flush_direct_mesh(target, clip, view, direct.take())?;
                self.meshes_msaa(target, transform, clip, &meshes[pending..index])?;
            }
            pending = index + 1;
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
                self.flush_direct_mesh(target, clip, Some(address), direct.take())?;
                direct = Some((&mesh.brush, geometry));
            }
        }
        self.flush_direct_mesh(target, clip, view, direct)?;
        if pending < meshes.len() {
            self.meshes_msaa(target, transform, clip, &meshes[pending..])?;
        }
        Ok(())
    }

    fn flush_direct_mesh(
        &mut self,
        target: &Image,
        clip: &Coverage,
        view: Option<Address>,
        direct: Option<(&batch::Brush, compiler::Tessellation)>,
    ) -> Result<(), Status> {
        let Some((brush, geometry)) = direct else {
            return Ok(());
        };
        let address = view.ok_or(Status::BadScene)?;
        let (page, paint) = self.canvas_paint(brush)?;
        let globals = self.globals(address, Extra::Paint(page));
        let mask = self.mask_group(clip);
        let material = self.resolve_binding(mask)?;
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            single(target.format),
            PipelineKind::Mesh,
            BlendKey::SrcOver,
        )?;
        let arena = &mut self.compositor.arena;
        let vertices = arena.vertices(self.gpu, &geometry.vertices)?;
        let indices = arena.vertices(self.gpu, &geometry.indices)?;
        let paint = arena.vertices(self.gpu, &[paint])?;
        self.compositor.render_stats.mesh_upload_bytes += vertices.size() + indices.size();
        let count = draw_count(geometry.indices.len())?;
        let scissor = target.local(clip.scissor.unwrap_or_else(|| target.area()));
        let pass = self.pending_pass(target);
        pass.list.indexed(
            &DrawPipeline::Standard(pipeline),
            &globals,
            &material,
            scissor,
            GpuMeshBuffer { vertices, indices },
            paint,
            0..count,
        )?;
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
        let Some(first) = meshes.first() else {
            return Ok(());
        };
        let Some(canvas) = self.canvas_target(target, transform, clip, meshes) else {
            return Ok(());
        };
        let blend =
            BlendKey::from_mode(batch::canvas_composite(first.composite)).unwrap_or_default();
        let CanvasTarget {
            resolved,
            multisample,
            mask_target,
            ..
        } = &canvas;
        let uncovered = Coverage::default();
        let view = self.transform(resolved, transform, &uncovered)?;
        let pipeline = self.compositor.pipelines.get(
            self.gpu,
            Target {
                format: resolved.format,
                samples: multisample.as_ref().map_or(1, |image| image.samples),
            },
            PipelineKind::Mesh,
            BlendKey::SrcOver,
        )?;
        let mut commands = self.compositor.draw_lists.pop().unwrap_or_default();
        let scissor = [0, 0, resolved.logical[0], resolved.logical[1]];
        let mut draws = 0;
        for mesh in meshes.iter().filter(|mesh| !mesh.indices.is_empty()) {
            let (page, paint) = self.canvas_paint(&mesh.brush)?;
            let globals = self.globals(view, Extra::Paint(page));
            let arena = &mut self.compositor.arena;
            let geometry = arena.mesh(self.gpu, mesh.buffer.get().ok_or(Status::BadScene)?)?;
            let paint = arena.vertices(self.gpu, &[paint])?;
            commands.indexed(
                &DrawPipeline::Standard(pipeline),
                &globals,
                &self.compositor.white_mask_group,
                scissor,
                geometry,
                paint,
                mesh.indices.clone(),
            )?;
            draws += 1;
        }
        self.flush_draws();
        self.clears.remove(&resolved.view);
        self.count_pass(&resolved.view);
        self.compositor.render_stats.draws += draws;
        let kind = if *mask_target {
            GpuPassKind::ClipMask
        } else if multisample.is_some() {
            GpuPassKind::CanvasMsaa
        } else {
            GpuPassKind::OffscreenEffect
        };
        {
            let stamps = pass_timestamps(&mut self.capture, kind, draws);
            let mut pass = self.command.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impasto Canvas mesh resolve"),
                timestamp_writes: stamps,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: multisample
                        .as_ref()
                        .map_or(&resolved.view, |image| &image.view),
                    depth_slice: None,
                    resolve_target: multisample.as_ref().map(|_| &resolved.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: if multisample.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                ..Default::default()
            });
            commands.encode(
                &mut pass,
                &resolved.view,
                resolved.size,
                &self.compositor.pipelines,
            );
        }
        commands.clear();
        self.compositor.draw_lists.push(commands);
        self.composite_canvas(target, clip, canvas, blend)
    }

    fn canvas_target(
        &mut self,
        target: &Image,
        transform: Affine,
        clip: &Coverage,
        meshes: &[&Mesh],
    ) -> Option<CanvasTarget> {
        let mut bounds = PictureBounds::Empty;
        for mesh in meshes {
            bounds = bounds.union(PictureBounds::rect(mesh.bounds));
        }
        let mut area = self.area(bounds.transform(transform), target)?;
        if let Some(clip_area) = clip.scissor {
            area = intersect_pixels(area, clip_area)?;
        }
        let mask_target = target.format == wgpu::TextureFormat::R8Unorm;
        let (format, resolved) = if mask_target {
            area = target.area();
            (target.format, target.clone())
        } else {
            let format = self.working;
            (format, self.take_area(area, format, 1))
        };
        let samples = sample_count(self.gpu, format);
        let multisample = (samples > 1).then(|| self.take_attachment(area, format, samples));
        Some(CanvasTarget {
            area,
            resolved,
            multisample,
            mask_target,
        })
    }

    fn canvas_paint(&mut self, brush: &batch::Brush) -> Result<(PaintPage, RectVertex), Status> {
        let mut paint = compiler::solid_rect([0.0; 4], brush.solid());
        let stops = if let batch::Brush::Gradient(gradient) = brush {
            set_rect_gradient(&mut paint, gradient);
            gradient.stops.as_slice()
        } else {
            &[]
        };
        let stops = self.stops(stops)?;
        let (page, index) = self.paint(stops, paint.extra(FrameArena::stop_index(stops)))?;
        Ok((page, paint.vertex(index)))
    }

    fn composite_canvas(
        &mut self,
        target: &Image,
        clip: &Coverage,
        canvas: CanvasTarget,
        blend: BlendKey,
    ) -> Result<(), Status> {
        let CanvasTarget {
            area,
            resolved,
            multisample,
            mask_target,
        } = canvas;
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
        Ok(())
    }
}

pub(super) fn sample_count(gpu: &Gpu, format: wgpu::TextureFormat) -> u32 {
    let features = format.guaranteed_format_features(gpu.device.features());
    if features.flags.contains(
        wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
            | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
    ) {
        4
    } else {
        1
    }
}
