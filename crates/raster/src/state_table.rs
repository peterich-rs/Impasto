//! Compact per-instance geometry state for adapters with vertex and fragment storage.

use crate::batch::{GlyphVertex, ImageVertex, RectVertex};
use crate::pipelines::ViewUniform;

/// Affine columns, local shape scale, and antialiasing width.
pub(crate) fn transform(view: &ViewUniform) -> [[f32; 4]; 2] {
    let [a, b, ..] = view.transform[0];
    let [c, d, ..] = view.transform[1];
    let [x, y, ..] = view.transform[3];
    [[a, b, c, d], [x, y, view.meta[0], view.target_info[2]]]
}

/// Clip coordinates are relative to the attachment, like fragment positions.
/// A zero-length entry has the reserved clip index zero.
pub(crate) fn clip(view: &ViewUniform, scissor: Option<[u32; 4]>) -> Vec<[f32; 4]> {
    let [_, count, mask, _] = view.meta;
    if count == 0.0 && mask == 0.0 && scissor.is_none() {
        return Vec::new();
    }
    let mut words = Vec::with_capacity(12);
    let [ox, oy, ..] = view.target_info;
    let [x, y, w, h] = scissor.unwrap_or([0, 0, 65536, 65536]);
    let [mx, my, sx, sy] = view.mask_rect;
    words.extend([
        [count, mask, f32::from(scissor.is_some()), 0.0],
        [x as f32, y as f32, (x + w) as f32, (y + h) as f32],
        [mx - ox, my - oy, sx, sy],
        view.clip_curves,
    ]);
    for (index, ([x, y, w, h], radii)) in view.clip_boxes.iter().zip(view.clip_radii).enumerate() {
        if index as f32 >= count {
            break;
        }
        words.extend([[x - ox, y - oy, *w, *h], radii]);
    }
    words
}

pub(crate) trait TableVertex: bytemuck::Pod {
    fn with_state(self, id: u32) -> Option<Self>;
    fn local_rect(&self) -> [f32; 4];

    fn bounds(&self, _view: &ViewUniform) -> [f32; 4] {
        self.local_rect()
    }
    fn opaque_core(self, _view: &ViewUniform, _size: [u32; 2]) -> Option<(Self, f32, bool)> {
        None
    }
}

impl TableVertex for RectVertex {
    fn opaque_core(
        mut self,
        view: &ViewUniform,
        [width, height]: [u32; 2],
    ) -> Option<(Self, f32, bool)> {
        let [a, b, ..] = view.transform[0];
        let [c, d, ..] = view.transform[1];
        if self.color[3] != 1.0
            || self.opacity != 1.0
            || self.shape[1..] != [0.0; 3]
            || self.paint != u32::MAX
            || a <= 0.0
            || d >= 0.0
            || b != 0.0
            || c != 0.0
            || view.target_info[2] <= 0.0
        {
            return None;
        }
        let [tx, ty, ..] = view.transform[3];
        let [x, y, w, h] = self.rect;
        let sx = width as f32 * 0.5;
        let sy = height as f32 * 0.5;
        let aligned = self.shape[0] == 0.0
            && a * sx <= view.target_info[2].recip() + 1e-4
            && -d * sy <= view.target_info[2].recip() + 1e-4
            && [
                (a * x + tx + 1.0) * sx,
                (a * (x + w) + tx + 1.0) * sx,
                (1.0 - (d * y + ty)) * sy,
                (1.0 - (d * (y + h) + ty)) * sy,
            ]
            .into_iter()
            .all(|edge| (edge - edge.round()).abs() <= 1e-4);
        if !aligned {
            let radius = self.shape[0].min(w.min(h) * 0.5);
            let inset = radius * (1.0 - std::f32::consts::FRAC_1_SQRT_2) + view.target_info[2];
            self.rect = [x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset];
        }
        let [_, _, w, h] = self.rect;
        let area = w * h * a * sx * -d * sy;
        (w > 0.0 && h > 0.0 && area.is_finite()).then_some((self, area, aligned))
    }

    fn with_state(mut self, id: u32) -> Option<Self> {
        self.padding[0] = id;
        Some(self)
    }

    fn local_rect(&self) -> [f32; 4] {
        self.rect
    }

    fn bounds(&self, view: &ViewUniform) -> [f32; 4] {
        let [x, y, w, h] = self.rect;
        let margin = self.shape[1] * 0.5 + 1.0 / view.meta[0].max(0.001);
        [x - margin, y - margin, w + 2.0 * margin, h + 2.0 * margin]
    }
}

impl TableVertex for ImageVertex {
    fn opaque_core(
        self,
        view: &ViewUniform,
        [width, height]: [u32; 2],
    ) -> Option<(Self, f32, bool)> {
        let [a, b, ..] = view.transform[0];
        let [c, d, ..] = view.transform[1];
        let [_, _, w, h] = self.rect;
        let area = w * h * (a * d - b * c).abs() * (width as f32 * height as f32 * 0.25);
        (self.color[3] == 1.0
            && self.meta[0] & crate::batch::IMAGE_OPAQUE != 0
            && area.is_finite()
            && area > 0.0)
            .then_some((self, area, true))
    }

    fn with_state(mut self, id: u32) -> Option<Self> {
        self.meta[1] = id;
        Some(self)
    }

    fn local_rect(&self) -> [f32; 4] {
        self.rect
    }
}

impl TableVertex for GlyphVertex {
    fn with_state(mut self, id: u32) -> Option<Self> {
        let paint = if self.paint == u32::MAX {
            u16::MAX
        } else {
            let paint = u16::try_from(self.paint).ok()?;
            if paint == u16::MAX {
                return None;
            }
            paint
        };
        self.paint = u32::from(paint) | (id & 0xffff_0000);
        self.flags_and_layer |= (id & 65535) << 16;
        Some(self)
    }

    fn local_rect(&self) -> [f32; 4] {
        self.rect
    }
}
