//! Scissor, analytic clip, and stencil or texture mask coverage.

use super::{
    Coverage, Frame, FullPass, Image, MASK_LIMIT, MaskKey, MaskMeshKey, MaskShapeKey, intersect,
    intersect_pixels,
};
use crate::Status;
use crate::batch;
use crate::compiler::{self, Affine, Clip, ClipShape, PictureBounds};

impl Frame<'_> {
    pub(super) fn coverage(&mut self, target: &Image, clip: &Clip) -> Result<Coverage, Status> {
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

    pub(super) fn area(&self, bounds: PictureBounds, target: &Image) -> Option<[u32; 4]> {
        let target_area = target.area();
        match bounds {
            PictureBounds::Empty => None,
            PictureBounds::Unbounded => Some(target_area),
            PictureBounds::Rect(bounds) => scissor(bounds, self.scene.scale, self.size)
                .and_then(|area| intersect_pixels(area, target_area)),
        }
    }
}

pub(super) fn add_rect_clip(clip: &mut Clip, rect: [f32; 4], radius: f32, transform: Affine) {
    add_corner_clip(clip, rect, radius, None, transform);
}

pub(super) fn add_corner_clip(
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
        });
    }
}

pub(super) fn merge_clip(parent: &Clip, child: &Clip, transform: Affine) -> Clip {
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
pub(super) fn scissor(rect: [f32; 4], scale: f32, size: [u32; 2]) -> Option<[u32; 4]> {
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
