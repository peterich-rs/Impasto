//! Corner geometry shared by scene paint and clipping.

/// Circular or continuous corners in top-left, top-right, bottom-right,
/// bottom-left order. A curve of 2 is circular. Higher values flatten the
/// curvature at the junction with the straight edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerShape {
    pub radii: [f32; 4],
    pub curve: f32,
}

impl CornerShape {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "fitted radii remain in the original finite f32 range after scaling"
    )]
    pub fn fitted(self, width: f32, height: f32) -> Self {
        let [tl, tr, br, bl] = self.radii.map(|radius| {
            if radius.is_finite() {
                f64::from(radius.max(0.0))
            } else {
                0.0
            }
        });
        let mut scale = 1.0_f64;
        for (length, pair) in [
            (width, tl + tr),
            (height, tr + br),
            (width, br + bl),
            (height, bl + tl),
        ] {
            if pair > 0.0 {
                scale = scale.min(f64::from(length.max(0.0)) / pair);
            }
        }
        Self {
            radii: [tl, tr, br, bl].map(|radius| (radius * scale) as f32),
            curve: if self.curve.is_finite() {
                self.curve.clamp(2.0, 8.0)
            } else {
                2.0
            },
        }
    }

    pub fn contains(self, size: [f32; 2], point: [f32; 2]) -> bool {
        let [w, h] = size;
        let [x, y] = point;
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            return false;
        }
        let shape = self.fitted(w, h);
        for (radius, [dx, dy]) in
            shape
                .radii
                .into_iter()
                .zip([[x, y], [w - x, y], [w - x, h - y], [x, h - y]])
        {
            if radius > 0.0 && dx < radius && dy < radius {
                let inside = (1.0 - dx / radius).powf(shape.curve)
                    + (1.0 - dy / radius).powf(shape.curve)
                    <= 1.0;
                if !inside {
                    return false;
                }
            }
        }
        true
    }
}
