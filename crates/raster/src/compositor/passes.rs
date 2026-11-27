//! Ordered draw encoding and lifetimes for frame-local attachments.

use super::{
    Attachment, CompositeDraw, Coverage, DrawList, DrawPipeline, Frame, Image, PassBinding,
    draw_count, extent,
};
use crate::Status;
use crate::arena::{DynamicGroup, FrameArena};
use crate::gpu::Gpu;
use crate::pipelines::PipelineCache;
use crate::profiler::{Capture, GpuPassKind, GpuPassSource};

pub(super) struct DrawPass {
    format: wgpu::TextureFormat,
    target: Attachment,
    size: [u32; 2],
    pub(super) list: DrawList,
    pub(super) draws: u32,
    pub(super) shader: Option<engine::scene::ShaderProgramId>,
}

#[derive(Default)]
pub(super) struct PassInputs {
    reads: std::collections::HashSet<Attachment>,
    retired: Vec<Image>,
}

impl PassInputs {
    pub(super) fn read(&mut self, view: Attachment) {
        self.reads.insert(view);
    }
}

pub(super) struct DeferredRootPass {
    pub(super) list: DrawList,
    size: [u32; 2],
    load: wgpu::LoadOp<wgpu::Color>,
    stamps: Option<PassStamps>,
}

impl DeferredRootPass {
    pub(super) fn encode(
        &self,
        command: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        pipelines: &PipelineCache,
    ) {
        let stamps = self.stamps.as_ref().map(PassStamps::render);
        let mut pass = render(command, output, self.load, stamps);
        self.list.encode(&mut pass, output, self.size, pipelines);
    }
}

pub(super) struct PassStamps {
    pub(super) queries: wgpu::QuerySet,
    pub(super) begin: Option<u32>,
    pub(super) end: Option<u32>,
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
    pub(super) fn render(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: self.begin,
            end_of_pass_write_index: self.end,
        }
    }
}

pub(super) fn frame_encoder(gpu: &Gpu) -> wgpu::CommandEncoder {
    gpu.device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("impasto frame"),
        })
}

pub(super) fn pass_timestamps<'a>(
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

impl Frame<'_> {
    #[expect(
        clippy::too_many_arguments,
        reason = "an instance run and its render state"
    )]
    pub(super) fn record<T: bytemuck::Pod>(
        &mut self,
        target: &Image,
        pipeline: DrawPipeline,
        globals: DynamicGroup,
        material: impl Into<PassBinding>,
        mask: Option<PassBinding>,
        vertices: &[T],
        scissor: Option<[u32; 4]>,
        vertex_count: u32,
    ) -> Result<(), Status> {
        let material = self.resolve_binding(material.into())?;
        let mask = mask.map(|mask| self.resolve_binding(mask)).transpose()?;
        let limit = FrameArena::vertex_limit::<T>(self.gpu);
        if limit == 0 {
            return Err(Status::OutOfMemory);
        }
        let scissor = scissor.map_or([0, 0, target.logical[0], target.logical[1]], |area| {
            target.local(area)
        });
        for chunk in vertices.chunks(limit) {
            let slice = self.compositor.arena.vertices(self.gpu, chunk)?;
            let pass = self.pending_pass(target);
            let added = pass.list.instanced(
                &pipeline,
                &globals,
                &material,
                mask.as_ref(),
                scissor,
                slice,
                vertex_count,
                draw_count(chunk.len())?,
            )?;
            pass.draws += u32::from(added);
            self.compositor.render_stats.draws += u32::from(added);
        }
        Ok(())
    }

    pub(super) fn read_mask(&mut self, target: &Image, clip: &Coverage) {
        if target.view == self.main
            && let Some(mask) = &clip.mask
        {
            self.main_inputs.read(mask.view.clone());
        }
    }

    pub(super) fn pending_pass(&mut self, target: &Image) -> &mut DrawPass {
        if target.view != self.main
            && self
                .pending
                .as_ref()
                .is_some_and(|pass| pass.target != target.view)
        {
            self.flush_draws();
        }
        let slot = if target.view == self.main {
            &mut self.main_pass
        } else {
            &mut self.pending
        };
        slot.get_or_insert_with(|| DrawPass {
            format: target.format,
            target: target.view.clone(),
            size: target.size,
            list: self.compositor.draw_lists.pop().unwrap_or_default(),
            draws: 0,
            shader: None,
        })
    }

    pub(super) fn flush_draws(&mut self) {
        if let Some(batch) = self.pending.take() {
            self.encode_draws(batch);
        }
    }

    pub(super) fn flush_target(&mut self, view: &Attachment) {
        self.flush_draws();
        if *view == self.main {
            if let Some(batch) = self.main_pass.take() {
                self.encode_draws(batch);
            }
            self.main_inputs.reads.clear();
            for image in self.main_inputs.retired.drain(..) {
                self.compositor.offscreen_live_bytes -= image.bytes();
                self.compositor.pool.recycle(image);
            }
        }
    }

    fn encode_draws(&mut self, batch: DrawPass) {
        let kind = self.pass_kind(&batch.target, batch.format);
        self.encode_pass(batch, kind);
    }

    fn encode_pass(&mut self, batch: DrawPass, kind: GpuPassKind) {
        self.count_pass(&batch.target);
        let load = self.load(&batch.target);
        let stamps = self
            .capture
            .as_mut()
            .and_then(|(capture, slice)| {
                capture.tagged_pass(
                    if batch.shader.is_some() {
                        GpuPassKind::ShaderEffect
                    } else {
                        kind
                    },
                    *slice,
                    batch.draws,
                    batch.shader.map(GpuPassSource::ShaderEffect),
                )
            })
            .map(PassStamps::from);
        let mut pass = DeferredRootPass {
            list: batch.list,
            size: batch.size,
            load,
            stamps,
        };
        if self.late_acquire && batch.target == self.root {
            debug_assert!(
                self.deferred_root.is_none(),
                "the surface has one output pass"
            );
            self.deferred_root = Some(pass);
        } else {
            pass.encode(&mut self.command, &batch.target, &self.compositor.pipelines);
            pass.list.clear();
            if self.compositor.draw_lists.len() < 2 {
                self.compositor.draw_lists.push(pass.list);
            }
        }
    }

    pub(super) fn composite_pass(
        &mut self,
        target: &Image,
        kind: GpuPassKind,
        draw: CompositeDraw,
    ) -> Result<(), Status> {
        let mut list = self.compositor.draw_lists.pop().unwrap_or_default();
        list.composite(draw)?;
        self.encode_pass(
            DrawPass {
                format: target.format,
                target: target.view.clone(),
                size: target.size,
                list,
                draws: 1,
                shader: None,
            },
            kind,
        );
        Ok(())
    }

    pub(super) fn resolve_binding(
        &mut self,
        binding: PassBinding,
    ) -> Result<wgpu::BindGroup, Status> {
        match binding {
            PassBinding::Ready(group) => Ok(group),
            PassBinding::Sample(sample) => {
                let cache = sample.cache.as_deref().ok_or(Status::BadScene)?;
                let (group, created) = cache.get(
                    self.gpu,
                    &self.compositor.pipelines.layouts.material,
                    &sample.view,
                    if sample.nearest {
                        &self.compositor.nearest
                    } else {
                        &self.compositor.linear
                    },
                    sample.nearest,
                );
                self.compositor.render_stats.bind_groups_created += u32::from(created);
                Ok(group)
            }
        }
    }

    pub(super) fn pass_kind(&self, view: &Attachment, format: wgpu::TextureFormat) -> GpuPassKind {
        if *view == self.main {
            GpuPassKind::Main
        } else if format == wgpu::TextureFormat::R8Unorm {
            GpuPassKind::ClipMask
        } else {
            GpuPassKind::OffscreenEffect
        }
    }

    pub(super) fn count_pass(&mut self, target: &Attachment) {
        self.compositor.render_stats.passes += 1;
        self.compositor.render_stats.offscreen_passes += u32::from(target != &self.root);
    }

    fn load(&mut self, target: &Attachment) -> wgpu::LoadOp<wgpu::Color> {
        self.clears
            .remove(target)
            .map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear)
    }

    pub(super) fn clear(&mut self, view: &Attachment) {
        self.clears.insert(view.clone(), wgpu::Color::TRANSPARENT);
    }

    pub(super) fn finish(&mut self, view: &Attachment) {
        self.flush_target(view);
        if self.clears.contains_key(view) {
            let kind = self.pass_kind(view, wgpu::TextureFormat::Rgba8UnormSrgb);
            let size = if *view == self.root {
                self.size
            } else {
                [view.texture().width(), view.texture().height()]
            };
            let list = self.compositor.draw_lists.pop().unwrap_or_default();
            self.encode_pass(
                DrawPass {
                    format: view.texture().format(),
                    target: view.clone(),
                    size,
                    list,
                    draws: 0,
                    shader: None,
                },
                kind,
            );
        }
    }

    pub(super) fn take(&mut self, size: [u32; 2], format: wgpu::TextureFormat) -> Image {
        self.take_area([0, 0, size[0], size[1]], format, 1)
    }

    pub(super) fn take_area(
        &mut self,
        area: [u32; 4],
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Image {
        self.take_area_with_usage(area, format, samples, Image::usage(samples))
    }

    pub(super) fn take_area_with_usage(
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

    pub(super) fn take_attachment(
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

    pub(super) fn release(&mut self, image: Image) {
        if self.main_inputs.reads.contains(&image.view) {
            self.main_inputs.retired.push(image);
            return;
        }
        self.compositor.offscreen_live_bytes -= image.bytes();
        self.compositor.pool.recycle(image);
    }

    pub(super) fn copy_attachment(
        &mut self,
        source: &Image,
        target: &Image,
        area: [u32; 4],
    ) -> Result<(), Status> {
        let [x, y, w, h] = source.local(area);
        self.compositor.render_stats.copy_pixels += u64::from(w) * u64::from(h);
        self.command.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                origin: wgpu::Origin3d { x, y, z: 0 },
                ..source.view.texture().as_image_copy()
            },
            target.view.texture().as_image_copy(),
            extent([w, h]),
        );
        Ok(())
    }
}
