//! Single-pass opaque pre-draw and painter-ordered translucent replay.

use super::{
    Attachment, CompositeDraw, DirectMesh, DrawBindings, DrawCall, DrawPass, Frame, Image,
    ImmediateDraw, PhysicalAttachments, set_scissor, single,
};
use crate::{
    Status,
    batch::RectVertex,
    batcher::Bounds,
    pipelines::{self, BlendKey, PipelineKind, PipelineMode},
};

pub(super) struct DepthPass {
    pub(super) depth: Image,
    size: [u32; 2],
    draws: Vec<DepthDraw>,
    pub(super) reads: Vec<Attachment>,
}

struct DepthDraw {
    core: Option<ImmediateDraw>,
    ordered: Option<OrderedDepthDraw>,
}

enum OrderedDepthDraw {
    Immediate(ImmediateDraw),
    Composite(CompositeDraw),
    Mesh(DirectMesh),
}

impl OrderedDepthDraw {
    fn encode(&self, pass: &mut wgpu::RenderPass<'_>, [width, height]: [u32; 2], depth: f32) {
        let full = [0, 0, width, height];
        match self {
            Self::Composite(call) => {
                set_scissor(pass, Some(full));
                call.encode_depth_range(pass, [depth; 2]);
            }
            Self::Immediate(call) => {
                pass.set_viewport(0.0, 0.0, width as f32, height as f32, depth, depth);
                set_scissor(pass, Some(call.scissor.unwrap_or(full)));
                call.encode(pass);
            }
            Self::Mesh(call) => {
                pass.set_viewport(0.0, 0.0, width as f32, height as f32, depth, depth);
                set_scissor(pass, Some(call.scissor.unwrap_or(full)));
                call.encode(pass);
            }
        }
    }
}

impl DepthPass {
    pub(super) fn encode(
        &self,
        command: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
        stamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(depth_view) = self.depth.view.physical() else {
            return;
        };
        let mut pass = command.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("impasto main depth"),
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
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let [w, h] = self.size.map(|value| value as f32);
        let depth = |index: usize| 1.0 - (index + 1) as f32 / (self.draws.len() + 2) as f32;
        for (index, call) in self.draws.iter().enumerate().rev() {
            if let Some(core) = &call.core {
                let z = depth(index);
                pass.set_viewport(0.0, 0.0, w, h, z, z);
                core.encode(&mut pass);
            }
        }
        for (index, call) in self.draws.iter().enumerate() {
            if let Some(ordered) = &call.ordered {
                ordered.encode(&mut pass, self.size, depth(index));
            }
        }
    }
}

impl Frame<'_> {
    pub(super) fn resolve_depth(
        &mut self,
        batch: &mut DepthPass,
        physical: &PhysicalAttachments,
    ) -> Result<(), Status> {
        batch.depth.view = Attachment::Physical(physical.view(&batch.depth.view)?.clone());
        for draw in &mut batch.draws {
            if let Some(core) = &mut draw.core {
                self.resolve_bindings(&mut core.bindings, physical)?;
            }
            match &mut draw.ordered {
                Some(OrderedDepthDraw::Immediate(call)) => {
                    self.resolve_bindings(&mut call.bindings, physical)?
                }
                Some(OrderedDepthDraw::Composite(call)) => {
                    self.resolve_composite(call, physical)?
                }
                Some(OrderedDepthDraw::Mesh(call)) => {
                    self.resolve_binding(&mut call.material, physical)?
                }
                None => {}
            }
        }
        Ok(())
    }

    pub(super) fn global_depth(&self, target: &Attachment) -> bool {
        *target == self.root
            && *target == self.main
            && self.compositor.render_graph
            && self.compositor.pipelines.layouts.state.is_some()
            && self.damage_scissor.is_none()
            && (self.compositor.early_z
                || self.compositor.adaptive_early_z && self.compositor.transient)
    }

    pub(super) fn prepare_depth(
        &mut self,
        batch: &DrawPass,
    ) -> Result<Option<(DepthPass, u32)>, Status> {
        if !self.global_depth(&batch.target)
            || batch.size != self.size
            || batch.calls.len() > 65_535
        {
            return Ok(None);
        }
        let mut area = 0.0;
        for call in &batch.calls {
            match call {
                DrawCall::Table(call) => area += call.core.as_ref().map_or(0.0, |core| core.area),
                DrawCall::Composite(_) | DrawCall::Mesh(_) => {}
                DrawCall::Immediate(_) | DrawCall::Resident { .. } => return Ok(None),
            }
        }
        let [width, height] = batch.size;
        if area == 0.0
            || !self.compositor.early_z
                && (area < 0.35 * width as f32 * height as f32 || !has_overdraw(batch))
        {
            return Ok(None);
        }
        let mut draws = Vec::with_capacity(batch.calls.len());
        let mut extra_draws = 0;
        let mut instances = 0;
        for call in &batch.calls {
            let call = match call {
                DrawCall::Table(call) => call,
                DrawCall::Composite(call) => {
                    let pipeline = self.compositor.pipelines.get_variant(
                        self.gpu,
                        single(batch.format),
                        call.kind,
                        call.blend,
                        PipelineMode::LegacyDepthRead,
                    )?;
                    draws.push(DepthDraw {
                        core: None,
                        ordered: Some(OrderedDepthDraw::Composite(CompositeDraw {
                            pipeline,
                            ..call.clone()
                        })),
                    });
                    continue;
                }
                DrawCall::Mesh(call) => {
                    let pipeline = self.compositor.pipelines.get_variant(
                        self.gpu,
                        single(batch.format),
                        PipelineKind::Mesh,
                        BlendKey::SrcOver,
                        PipelineMode::LegacyDepthRead,
                    )?;
                    draws.push(DepthDraw {
                        core: None,
                        ordered: Some(OrderedDepthDraw::Mesh(DirectMesh {
                            pipeline,
                            ..call.clone()
                        })),
                    });
                    continue;
                }
                _ => return Ok(None),
            };
            let mut make = |bytes: &[u8], mode| -> Result<_, Status> {
                let pipeline = self.compositor.pipelines.get_variant(
                    self.gpu,
                    single(batch.format),
                    call.kind,
                    call.blend,
                    mode,
                )?;
                Ok(ImmediateDraw {
                    bindings: DrawBindings {
                        pipeline,
                        ..call.bindings.clone()
                    },
                    vertices: self.compositor.arena.vertices(self.gpu, bytes)?,
                    scissor: None,
                    instances: call.instances,
                })
            };
            let (core, ordered) = if let Some(core) = &call.core {
                let mut reversed = Vec::with_capacity(core.bytes.len());
                for rect in core
                    .bytes
                    .as_chunks::<{ size_of::<RectVertex>() }>()
                    .0
                    .iter()
                    .rev()
                {
                    reversed.extend_from_slice(rect);
                }
                let opaque = make(&reversed, PipelineMode::DepthCore)?;
                let fringe = (!core.complete)
                    .then(|| make(&call.bytes, PipelineMode::DepthFringe))
                    .transpose()?;
                extra_draws += u32::from(fringe.is_some());
                instances += call.instances;
                (Some(opaque), fringe)
            } else {
                (None, Some(make(&call.bytes, PipelineMode::DepthRead)?))
            };
            draws.push(DepthDraw {
                core,
                ordered: ordered.map(OrderedDepthDraw::Immediate),
            });
        }
        let depth = self.take_exact_attachment([0, 0, width, height], pipelines::EARLY_DEPTH);
        self.compositor.render_stats.early_z_instances += instances;
        self.compositor.render_stats.draws += extra_draws;
        Ok(Some((
            DepthPass {
                depth,
                size: batch.size,
                draws,
                reads: batch.reads.clone(),
            },
            batch.draws + extra_draws,
        )))
    }
}

/// Skip a depth attachment when opaque coverage mostly occupies different
/// screen regions. Work in whole cells so adjacent cards cannot count as
/// overlap solely because their bounds touch a bin boundary.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "grid coordinates are clamped"
)]
fn has_overdraw(batch: &DrawPass) -> bool {
    const SIDE: usize = 32;
    const MIN_OVERLAP_CELLS: u32 = (SIDE * SIDE / 10) as u32;
    let mut coverage = [0u32; SIDE];
    let mut overlap = [0u32; SIDE];
    let mut occluded = 0;
    let [width, height] = batch.size;
    let sx = SIDE as f32 / width as f32;
    let sy = SIDE as f32 / height as f32;
    for call in &batch.calls {
        if let DrawCall::Table(draw) = call
            && let Some(core) = &draw.core
        {
            for &Bounds([left, top, right, bottom]) in &core.bounds {
                let x0 = (left * sx).ceil().clamp(0.0, SIDE as f32) as usize;
                let y0 = (top * sy).ceil().clamp(0.0, SIDE as f32) as usize;
                let x1 = (right * sx).floor().clamp(0.0, SIDE as f32) as usize;
                let y1 = (bottom * sy).floor().clamp(0.0, SIDE as f32) as usize;
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let columns = (u32::MAX << x0) & (u32::MAX >> (SIDE - x1));
                for row in y0..y1 {
                    let new = coverage[row] & columns & !overlap[row];
                    overlap[row] |= new;
                    coverage[row] |= columns;
                    occluded += new.count_ones();
                    if occluded >= MIN_OVERLAP_CELLS {
                        return true;
                    }
                }
            }
        }
    }
    false
}
