//! Typed instance packing and spatial batching for the storage-table path.

use super::{
    DrawBindings, DrawCall, Frame, Image, OpaqueClass, OpaqueCore, TableDraw, draw_count, single,
};
use crate::{
    Status,
    arena::{Extra, FrameArena},
    batch::RectVertex,
    batcher::{Bounds, SpatialBatcher},
    pipelines::{BlendKey, PipelineKind, PipelineMode, ViewUniform},
    state_table::{self, TableVertex},
};

impl Frame<'_> {
    #[expect(
        clippy::too_many_arguments,
        reason = "records typed instances and their shared draw state"
    )]
    pub(super) fn record_instances<T: TableVertex>(
        &mut self,
        target: &Image,
        view: ViewUniform,
        scissor: Option<[u32; 4]>,
        kind: PipelineKind,
        blend: BlendKey,
        extra: Extra,
        material: wgpu::BindGroup,
        mask: Option<wgpu::BindGroup>,
        vertices: &[T],
        vertex_count: u32,
    ) -> Result<(), Status> {
        if vertices.is_empty() {
            return Ok(());
        }
        let mut scissor_local = scissor.map(|area| target.local(area));
        let table =
            self.compositor.pipelines.layouts.state.is_some() && kind != PipelineKind::TextLcd;
        if table {
            let bounds = instance_bounds(vertices, &view, target.size);
            let mut table_view = view;
            if let Some(bounds) = bounds {
                if scissor_local.is_some_and(|area| inside_scissor(bounds, area)) {
                    scissor_local = None;
                }
                if table_view.meta[2] == 0.0 && inside_analytic_clip(bounds, &table_view) {
                    table_view.meta[1] = 0.0;
                }
            }
            let kind = kind.with_clip(
                table_view.meta[1] != 0.0 || table_view.meta[2] != 0.0 || scissor_local.is_some(),
            );
            let (page, id) = self.compositor.arena.state(
                self.gpu,
                state_table::transform(&table_view),
                state_table::clip(&table_view, scissor_local),
            )?;
            if let Some(encoded) = vertices
                .iter()
                .map(|&vertex| vertex.with_state(id))
                .collect::<Option<Vec<_>>>()
            {
                let globals = self.compositor.arena.draw_group(
                    self.gpu,
                    &self.compositor.pipelines.layouts,
                    page,
                    extra,
                    true,
                );
                let pipeline = self.compositor.pipelines.get_variant(
                    self.gpu,
                    single(target.format),
                    kind,
                    blend,
                    PipelineMode::Table,
                )?;
                let bindings = DrawBindings {
                    pipeline,
                    globals,
                    material,
                    mask,
                    vertex_count,
                    opaque: OpaqueClass::Ordered,
                };
                let limit = FrameArena::vertex_limit::<T>(self.gpu);
                let max_bytes = limit * size_of::<T>();
                let spatial = self.compositor.spatial_batch && bounds.is_some();
                for chunk in encoded.chunks(limit) {
                    let instances = draw_count(chunk.len())?;
                    let bytes = bytemuck::cast_slice(chunk);
                    let core = if self.global_depth(&target.view)
                        && blend == BlendKey::SrcOver
                        && table_view.meta[1] == 0.0
                        && table_view.meta[2] == 0.0
                        && scissor_local.is_none()
                        && matches!(
                            kind,
                            PipelineKind::RectSolidUnclipped
                                | PipelineKind::ImageUnclipped
                                | PipelineKind::ImageArray
                                | PipelineKind::SmallImages
                        ) {
                        opaque_core(chunk, &table_view, target.size)
                    } else {
                        None
                    };
                    let bindings = DrawBindings {
                        opaque: match &core {
                            None => OpaqueClass::Ordered,
                            Some(core) if core.complete => OpaqueClass::Complete,
                            Some(_) => OpaqueClass::Fringe,
                        },
                        ..bindings.clone()
                    };
                    let pass = self.pending_pass(target);
                    let bounds = bounds.unwrap_or(Bounds([0.0, 0.0, f32::INFINITY, f32::INFINITY]));
                    let index = if spatial {
                        let calls = &pass.calls;
                        pass.spatial
                            .get_or_insert_with(|| Box::new(SpatialBatcher::new(target.size)))
                            .append(
                                bindings.clone(),
                                bounds,
                                calls.len(),
                                |candidate, refine| {
                                    let DrawCall::Table(batch) = &calls[candidate] else {
                                        return false;
                                    };
                                    if batch.bytes.len() + bytes.len() > max_bytes
                                        || bindings.opaque == OpaqueClass::Fringe
                                            && batch
                                                .bounds
                                                .iter()
                                                .any(|&other| bounds.overlaps(other))
                                    {
                                        return false;
                                    }
                                    !refine
                                        || calls.len() - candidate <= 64
                                            && calls[candidate + 1..].iter().all(
                                                |call| match call {
                                                    DrawCall::Table(batch) => !batch
                                                        .bounds
                                                        .iter()
                                                        .any(|&other| bounds.overlaps(other)),
                                                    _ => false,
                                                },
                                            )
                                },
                            )
                    } else {
                        pass.calls.len()
                    };
                    if let Some(DrawCall::Table(batch)) = pass.calls.get_mut(index) {
                        batch.bytes.extend_from_slice(bytes);
                        batch.instances += instances;
                        batch.core = match (batch.core.take(), core) {
                            (Some(mut combined), Some(next))
                                if combined.complete && next.complete
                                    || !batch
                                        .bounds
                                        .iter()
                                        .any(|&other| bounds.overlaps(other)) =>
                            {
                                combined.bytes.extend_from_slice(&next.bytes);
                                combined.area += next.area;
                                combined.bounds.extend(next.bounds);
                                combined.complete &= next.complete;
                                Some(combined)
                            }
                            _ => None,
                        };
                        batch.bounds.push(bounds);
                        self.compositor.render_stats.spatial_merges += 1;
                        continue;
                    }
                    if !spatial {
                        pass.barrier(pass.calls.len() + 1);
                    }
                    pass.calls.push(DrawCall::Table(TableDraw {
                        bindings,
                        bytes: bytes.to_vec(),
                        instances,
                        bounds: vec![bounds],
                        kind,
                        blend,
                        core,
                    }));
                    pass.draws += 1;
                    self.compositor.render_stats.draws += 1;
                }
                return Ok(());
            }
        }
        let view = self.uniform(&view)?;
        let globals = self.globals(view, extra);
        let pipeline =
            self.compositor
                .pipelines
                .get(self.gpu, single(target.format), kind, blend)?;
        self.record(
            target,
            pipeline,
            globals,
            material,
            mask,
            vertices,
            scissor,
            vertex_count,
        )
    }
}

fn project_bounds(
    rects: impl IntoIterator<Item = [f32; 4]>,
    view: &ViewUniform,
    [width, height]: [u32; 2],
) -> Option<Bounds> {
    let [a, b, ..] = view.transform[0];
    let [c, d, ..] = view.transform[1];
    let [tx, ty, ..] = view.transform[3];
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y, w, h] in rects {
        for (x, y) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            let px = (a * x + c * y + tx + 1.0) * width as f32 * 0.5;
            let py = (1.0 - (b * x + d * y + ty)) * height as f32 * 0.5;
            if !px.is_finite() || !py.is_finite() {
                return None;
            }
            bounds = [
                bounds[0].min(px),
                bounds[1].min(py),
                bounds[2].max(px),
                bounds[3].max(py),
            ];
        }
    }
    Some(Bounds(bounds))
}

fn instance_bounds<T: TableVertex>(
    vertices: &[T],
    view: &ViewUniform,
    size: [u32; 2],
) -> Option<Bounds> {
    let Bounds([x0, y0, x1, y1]) = project_bounds(
        vertices.iter().map(|vertex| vertex.bounds(view)),
        view,
        size,
    )?;
    Some(Bounds([x0.floor(), y0.floor(), x1.ceil(), y1.ceil()]))
}

fn inside_scissor(Bounds([x0, y0, x1, y1]): Bounds, [x, y, w, h]: [u32; 4]) -> bool {
    x0 >= x as f32 && y0 >= y as f32 && x1 <= (x + w) as f32 && y1 <= (y + h) as f32
}

fn inside_analytic_clip(Bounds([x0, y0, x1, y1]): Bounds, view: &ViewUniform) -> bool {
    let [ox, oy, ..] = view.target_info;
    let [x0, y0, x1, y1] = [x0 + ox, y0 + oy, x1 + ox, y1 + oy];
    view.clip_boxes.iter().zip(view.clip_radii).enumerate().all(
        |(index, (&[cx, cy, w, h], radii))| {
            if index as f32 >= view.meta[1] {
                return true;
            }
            let radius = radii.into_iter().fold(0.0, f32::max);
            let inset = radius + 1.0;
            x0 >= cx - w + inset
                && x1 <= cx + w - inset
                && y0 >= cy - h + inset
                && y1 <= cy + h - inset
        },
    )
}

fn opaque_core<T: TableVertex>(
    vertices: &[T],
    view: &ViewUniform,
    size: [u32; 2],
) -> Option<OpaqueCore> {
    let mut core = Vec::with_capacity(vertices.len());
    let mut area = 0.0;
    let mut bounds = Vec::with_capacity(vertices.len());
    let mut complete = true;
    for &vertex in vertices {
        let (vertex, estimate, whole) = vertex.opaque_core(view, size)?;
        bounds.push(project_bounds([vertex.local_rect()], view, size)?);
        core.push(vertex);
        area += estimate;
        complete &= whole;
    }
    // Overlapping rounded/fringed instances need a distinct depth per instance.
    if !complete && vertices.len() != 1 {
        return None;
    }
    Some(OpaqueCore {
        bytes: bytemuck::cast_slice(&core).to_vec(),
        bounds,
        area,
        complete,
    })
}

/// Distinct depth is required where antialiased fringes can overlap another
/// instance. Completely opaque rects can share a batch and use reverse order.
pub(super) fn same_depth_class(
    a: RectVertex,
    b: RectVertex,
    view: &ViewUniform,
    size: [u32; 2],
) -> bool {
    match (a.opaque_core(view, size), b.opaque_core(view, size)) {
        (None, None) => true,
        (Some((_, _, a)), Some((_, _, b))) => a && b,
        _ => false,
    }
}

pub(super) fn large_opaque_core(
    vertices: &[RectVertex],
    view: &ViewUniform,
    size: [u32; 2],
) -> bool {
    let threshold = 0.35 * size[0] as f32 * size[1] as f32;
    vertices.iter().copied().any(|rect| {
        rect.opaque_core(view, size)
            .is_some_and(|(_, area, _)| area >= threshold)
    })
}
