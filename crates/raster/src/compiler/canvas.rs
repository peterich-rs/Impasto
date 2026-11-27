//! Canvas command replay and path tessellation during picture compilation.

use std::sync::Arc;

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use engine::scene::{
    CanvasBrush, CanvasComposite, CanvasImageOp, CanvasIsolation, CanvasList, CanvasOp,
    CanvasTextOp,
};

use super::{
    ClipShape, Compiler, Draw, Effect, RunPaint, append_glyphs, compose, mesh_bounds, positive_box,
    scale_color, solid_rect,
};
use crate::batch::{self, Brush, ImageInstance, Mesh, pack_canvas_gradient};
use crate::color::linear_premultiplied;
use crate::pipelines::BlendKey;
use crate::status::Status;
use crate::textures::Load;

use super::path::{PreparedPath, prepare_path};

impl Compiler<'_> {
    pub(super) fn canvas(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        list: &CanvasList,
    ) -> Result<(), Status> {
        if list.ops().is_empty() || !positive_box(x, y, width, height) {
            return Ok(());
        }
        let entry = self.stack.len();
        let isolated = list.isolation() == CanvasIsolation::Isolated;
        if isolated {
            self.push_effect(Effect::Isolation, 1.0, 0.0, None);
        }
        self.push_clip(x, y, width, height, 0.0, None);
        self.push_transform(x, y, 0.0, 1.0, 1.0);
        let floor = self.stack.len();
        let ops = list.ops();
        if let Some(pool) = self.pool.filter(|_| crate::prepare::path_count(ops) >= 16) {
            // Canvas paths already contain their local transforms. Save/restore
            // and the remaining ops leave the enclosing scale unchanged.
            let tolerance = self.tessellation_tolerance();
            let prepared: Vec<_> = pool.install(|| {
                ops.par_iter()
                    .map(|op| match op {
                        CanvasOp::Clip { path } if path.verbs.is_empty() => Ok(None),
                        CanvasOp::Fill { path, .. } | CanvasOp::Vector { path, .. } => {
                            prepare_path(path, tolerance, true).map(Some)
                        }
                        CanvasOp::Clip { path } => prepare_path(path, tolerance, false).map(Some),
                        _ => Ok(None),
                    })
                    .collect()
            });
            for (op, buffers) in ops.iter().zip(prepared) {
                self.canvas_op(op, floor, buffers?)?;
            }
        } else {
            for op in ops {
                let buffers = match op {
                    CanvasOp::Fill { path, .. } | CanvasOp::Vector { path, .. } => {
                        Some(prepare_path(path, self.tessellation_tolerance(), true)?)
                    }
                    CanvasOp::Clip { path } if !path.verbs.is_empty() => {
                        Some(prepare_path(path, self.tessellation_tolerance(), false)?)
                    }
                    _ => None,
                };
                self.canvas_op(op, floor, buffers)?;
            }
        }
        self.stack.truncate(entry);
        self.boundary();
        Ok(())
    }

    fn mesh(
        &mut self,
        brush: Brush,
        composite: CanvasComposite,
        prepared: PreparedPath,
    ) -> Result<Mesh, Status> {
        let PreparedPath { buffers, fringe } = prepared;
        let base = u32::try_from(self.vertices.len()).map_err(|_| Status::BadScene)?;
        let start = u32::try_from(self.indices.len()).map_err(|_| Status::BadScene)?;
        let bounds = mesh_bounds(&buffers.vertices);
        for index in buffers.indices {
            self.indices
                .push(base.checked_add(index).ok_or(Status::BadScene)?);
        }
        let end = u32::try_from(self.indices.len()).map_err(|_| Status::BadScene)?;
        self.vertices.extend(buffers.vertices);
        Ok(Mesh {
            buffer: Arc::clone(&self.meshes),
            indices: start..end,
            fringe,
            bounds,
            brush,
            composite,
        })
    }

    fn canvas_op(
        &mut self,
        op: &CanvasOp,
        floor: usize,
        buffers: Option<PreparedPath>,
    ) -> Result<(), Status> {
        match op {
            CanvasOp::Save => {
                self.stack.push(self.current().clone());
                self.boundary();
            }
            CanvasOp::Restore => {
                if self.stack.len() > floor {
                    self.pop();
                }
            }
            CanvasOp::Clip { path } => {
                let transform = self.current().transform;
                if path.verbs.is_empty() {
                    let clip = &mut self.current_mut().clip;
                    clip.scissor = Some([0.0; 4]);
                    clip.hard_scissor = Some([0.0; 4]);
                } else {
                    let mesh = self.mesh(
                        Brush::Solid([1.0; 4]),
                        CanvasComposite::SourceOver,
                        buffers.ok_or(Status::BadScene)?,
                    )?;
                    self.current_mut().clip.shapes.push(ClipShape {
                        rect: mesh.bounds,
                        radius: 0.0,
                        corners: None,
                        transform,
                        mesh: Some(Arc::new(mesh)),
                    });
                }
                self.boundary();
            }
            CanvasOp::Fill { paint, .. } | CanvasOp::Vector { paint, .. } => {
                let brush = match &paint.brush {
                    CanvasBrush::Color(color) => {
                        Brush::Solid(linear_premultiplied(*color, paint.alpha).unwrap_or([0.0; 4]))
                    }
                    gradient => Brush::Gradient(
                        pack_canvas_gradient(gradient, paint.alpha).ok_or(Status::BadScene)?,
                    ),
                };
                let mesh = self.mesh(brush, paint.composite, buffers.ok_or(Status::BadScene)?)?;
                let isolated = paint.composite == CanvasComposite::Multiply
                    && self.push_canvas_composite(paint.composite);
                self.push_draw(Draw::Mesh(mesh));
                if isolated {
                    self.pop();
                }
            }
            CanvasOp::SolidRect {
                bounds,
                radius,
                color,
                alpha,
            } => {
                let [x, y, w, h] = *bounds;
                if positive_box(x, y, w, h)
                    && let Some(color) = linear_premultiplied(*color, *alpha)
                {
                    let mut instance = solid_rect(*bounds, color);
                    instance.radius = radius.clamp(0.0, w.min(h) * 0.5);
                    self.push_rects([instance], None, BlendKey::SrcOver);
                }
            }
            CanvasOp::Text(text) => self.canvas_text(text),
            CanvasOp::Image(image) => self.canvas_image(image)?,
            CanvasOp::Clear { x, y, w, h, ctm } => {
                let mut state = self.current().clone();
                state.transform = compose(state.transform, *ctm);
                self.stack.push(state);
                self.push_rects(
                    [solid_rect([*x, *y, *w, *h], [1.0; 4])],
                    None,
                    BlendKey::DstOut,
                );
                self.pop();
            }
        }
        Ok(())
    }

    fn canvas_text(&mut self, text: &CanvasTextOp) {
        if let Some(shaped) = &text.shaped {
            let Some(paint) = canvas_brush(&text.paint.brush, text.paint.alpha, 1.0) else {
                return;
            };
            let isolated = self.push_canvas_composite(text.paint.composite);
            let mut state = self.current().clone();
            let squeeze = [
                text.compress,
                0.0,
                0.0,
                1.0,
                text.anchor_x * (1.0 - text.compress),
                0.0,
            ];
            let transform = compose(state.transform, compose(text.ctm, squeeze));
            let transformed = state.transform != transform;
            if transformed {
                state.transform = transform;
                self.stack.push(state);
            }
            let mut glyphs = Vec::new();
            for run in &shaped.runs {
                append_glyphs(&mut glyphs, run, text.x, text.y, &paint);
            }
            if !glyphs.is_empty() {
                self.push_draw(Draw::Glyphs(glyphs));
            }
            if transformed {
                self.pop();
            }
            if isolated {
                self.pop();
            }
        }
    }

    fn canvas_image(&mut self, image: &CanvasImageOp) -> Result<(), Status> {
        let load = (self.load)(&image.source);
        match load {
            Load::Ready {
                width: tex_w,
                height: tex_h,
                ..
            } => {
                let Some((rect, uv)) = canvas_image_sample(image, tex_w, tex_h) else {
                    return Ok(());
                };
                let blend = BlendKey::from_mode(batch::canvas_composite(image.composite));
                let isolated = blend.is_none() && self.push_canvas_composite(image.composite);
                let mut state = self.current().clone();
                state.transform = compose(state.transform, image.ctm);
                self.stack.push(state);
                let alpha = f32::from(image.alpha) / 255.0;
                self.remember_image(&image.source, false);
                self.push_draw(Draw::Images {
                    source: Arc::clone(&image.source),
                    blend: blend.unwrap_or_default(),
                    instances: vec![ImageInstance {
                        rect,
                        uv,
                        color: [alpha, alpha, alpha, alpha],
                        flags: batch::IMAGE_BASE_LEVEL
                            | if image.smoothing {
                                0
                            } else {
                                batch::IMAGE_NEAREST
                            },
                    }],
                });
                self.pop();
                if isolated {
                    self.pop();
                }
            }
            Load::Missing => {
                if !image
                    .source
                    .starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX)
                {
                    self.waiting.push(Arc::clone(&image.source));
                }
            }
            Load::Empty => self.waiting.push(Arc::clone(&image.source)),
            Load::Failed => return Err(Status::BadScene),
        }
        Ok(())
    }
}

fn canvas_brush(brush: &CanvasBrush, alpha: u8, opacity: f32) -> Option<RunPaint> {
    let coverage = crate::color::coverage_alpha(alpha, opacity);
    match brush {
        CanvasBrush::Color(color) => Some(RunPaint {
            solid: linear_premultiplied(*color, alpha)?,
            gradient: None,
        })
        .map(|mut paint| {
            paint.solid = scale_color(paint.solid, opacity);
            paint
        }),
        gradient => {
            let packed = pack_canvas_gradient(gradient, alpha)?;
            let solid = [0.0, 0.0, 0.0, coverage];
            Some(RunPaint {
                solid,
                gradient: Some(Arc::new(packed)),
            })
        }
    }
}

fn canvas_image_sample(
    image: &CanvasImageOp,
    width: f32,
    height: f32,
) -> Option<([f32; 4], [f32; 4])> {
    let [x, y, w, h] = image.src.unwrap_or([0.0, 0.0, width, height]);
    let [dw, dh] = [image.w.unwrap_or(w), image.h.unwrap_or(h)];
    if !positive_box(x, y, w, h) || !positive_box(image.x, image.y, dw, dh) {
        return None;
    }
    let left = x.max(0.0);
    let top = y.max(0.0);
    let right = (x + w).min(width);
    let bottom = (y + h).min(height);
    if right <= left || bottom <= top {
        return None;
    }
    let rect = [
        image.x + (left - x) * dw / w,
        image.y + (top - y) * dh / h,
        (right - left) * dw / w,
        (bottom - top) * dh / h,
    ];
    let uv = [left / width, top / height, right / width, bottom / height];
    Some((rect, uv))
}
