//! A coverage ramp for a single convex Canvas contour.

use engine::scene::CanvasPath;
use lyon_path::{Event, iterator::PathIterator};

use crate::{
    batch,
    compiler::{Affine, Tessellation},
};

pub(crate) fn convex(path: &CanvasPath, tolerance: f32) -> Option<Box<[[f32; 2]]>> {
    let geometry = batch::tessellation_path(&path.verbs);
    let mut points = Vec::new();
    for event in geometry.iter().flattened(tolerance) {
        match event {
            Event::Begin { at } if points.is_empty() => points.push([at.x, at.y]),
            Event::Line { to, .. } => {
                if points.last() != Some(&[to.x, to.y]) {
                    if points.len() > 96 {
                        return None;
                    }
                    points.push([to.x, to.y]);
                }
            }
            Event::End { .. } => {}
            _ => return None,
        }
    }
    if points.first() == points.last() {
        points.pop();
    }
    if !(3..=96).contains(&points.len()) || !points.iter().flatten().all(|p| p.is_finite()) {
        return None;
    }

    // Each edge must support the entire contour. This also excludes star
    // polygons whose local turns all have the same sign.
    let mut winding = 0.0;
    for i in 0..points.len() {
        let start = points[i];
        // A contour that traverses the same boundary twice still satisfies
        // the supporting-edge test. It needs the normal winding fill.
        if points[..i].contains(&start) {
            return None;
        }
        let end = points[(i + 1) % points.len()];
        let direction = sub(end, start);
        for &point in &points {
            let side = cross(direction, sub(point, start));
            if side != 0.0 {
                if winding == 0.0 {
                    winding = side;
                } else if side * winding < 0.0 {
                    return None;
                }
            }
        }
    }
    if winding == 0.0 {
        return None;
    }
    if winding < 0.0 {
        points.reverse();
    }
    Some(points.into_boxed_slice())
}

pub(crate) fn mesh(points: &[[f32; 2]], transform: Affine, scale: f32) -> Option<Tessellation> {
    let [a, b, c, d, ..] = transform;
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() < 1e-6 || scale <= 0.0 {
        return None;
    }
    let normal = |delta: [f32; 2]| {
        let [x, y] = delta;
        let [x, y] = [a * x + c * y, b * x + d * y];
        let length = x.hypot(y);
        [y / length, -x / length]
    };
    let size = points.len();
    let mut vertices = Vec::with_capacity(size * 2);
    for (i, &[x, y]) in points.iter().enumerate() {
        let previous = normal(sub(points[i], points[(i + size - 1) % size]));
        let next = normal(sub(points[(i + 1) % size], points[i]));
        let miter = 1.0 + previous[0] * next[0] + previous[1] * next[1];
        if !miter.is_finite() || miter < 0.125 {
            return None;
        }
        let distance = determinant.signum() * 0.5 / (miter * scale);
        let sx = (previous[0] + next[0]) * distance;
        let sy = (previous[1] + next[1]) * distance;
        let dx = (d * sx - c * sy) / determinant;
        let dy = (a * sy - b * sx) / determinant;
        vertices.extend([[x - dx, y - dy, 1.0], [x + dx, y + dy, 0.0]]);
    }

    // If the inset crossed the opposite side of a thin path, MSAA retains
    // subpixel detail better than forcing an opaque interior.
    for i in 0..size {
        let start = points[i];
        let direction = sub(points[(i + 1) % size], start);
        if vertices
            .iter()
            .step_by(2)
            .any(|&[x, y, _]| cross(direction, sub([x, y], start)) <= 0.0)
        {
            return None;
        }
        let [x, y, _] = vertices[2 * i];
        let [nx, ny, _] = vertices[2 * ((i + 1) % size)];
        let inner = sub([nx, ny], [x, y]);
        if direction[0] * inner[0] + direction[1] * inner[1] <= 0.0 {
            return None;
        }
    }

    let mut indices = Vec::with_capacity(size * 9);
    let size = u32::try_from(size).ok()?;
    for i in 0..size {
        let inner = i * 2;
        let next = ((i + 1) % size) * 2;
        if i > 0 && i + 1 < size {
            indices.extend([0, inner, next]);
        }
        indices.extend([inner, inner + 1, next + 1, inner, next + 1, next]);
    }
    Some(Tessellation { vertices, indices })
}

fn sub([x, y]: [f32; 2], [a, b]: [f32; 2]) -> [f32; 2] {
    [x - a, y - b]
}

fn cross([x, y]: [f32; 2], [a, b]: [f32; 2]) -> f32 {
    x * b - y * a
}
