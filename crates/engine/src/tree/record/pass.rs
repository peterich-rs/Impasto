//! Paint commands for one recorded node.

use std::sync::Arc;

use crate::scene::Command;

use super::super::node::{Node, Role};
use super::{PreparedGradient, RectPaint, positive_sigma, prepared_gradient, rect_paint};

pub(super) fn emit_rect(commands: &mut Vec<Command>, node: &Node, x: f32, y: f32) {
    let RectPaint {
        fill,
        stroke_width,
        stroke_color,
        gradient,
        shadow,
        box_shadows,
    } = rect_paint(node);
    if node.w <= 0.0
        || node.h <= 0.0
        || (fill.is_none()
            && stroke_width <= 0.0
            && gradient.is_none()
            && shadow.is_none()
            && box_shadows.is_empty())
    {
        return;
    }
    let mut color = fill.unwrap_or([0, 0, 0, 0]);
    let gradient = match gradient.as_ref().map(prepared_gradient) {
        Some(PreparedGradient::Paint(gradient)) => Some(gradient),
        Some(PreparedGradient::Solid(stop)) => {
            color = stop;
            None
        }
        None | Some(PreparedGradient::Drop) => None,
    };
    commands.push(Command::Rect {
        x,
        y,
        w: node.w,
        h: node.h,
        corner_radius: node.corner_radius(),
        color,
        stroke_width,
        stroke_color,
        gradient,
        shadow,
        box_shadows,
    });
}

pub(super) fn emit_outline(commands: &mut Vec<Command>, node: &Node, x: f32, y: f32) {
    let Some(paint) = node.paint.as_deref() else {
        return;
    };
    let width = paint.outline_width;
    if width <= 0.0 || node.w <= 0.0 || node.h <= 0.0 {
        return;
    }
    commands.push(Command::Rect {
        x: x - width * 0.5,
        y: y - width * 0.5,
        w: node.w + width,
        h: node.h + width,
        corner_radius: node.corner_radius() + width * 0.5,
        color: [0; 4],
        stroke_width: width,
        stroke_color: paint.outline_color,
        gradient: None,
        shadow: None,
        box_shadows: Arc::from([]),
    });
}

pub(super) fn emit_text(
    commands: &mut Vec<Command>,
    node: &Node,
    x: f32,
    y: f32,
    marks: &[(super::super::Rect, [u8; 4])],
) {
    let Some(text) = node.text() else {
        return;
    };
    if text.utf8.is_empty() {
        return;
    }
    let clip_overflow = text.style.overflow == crate::scene::TextOverflow::Clip
        && !text.style.soft_wrap
        && node.w > 0.0
        && node.h > 0.0;
    if clip_overflow {
        commands.push(Command::PushClip {
            x,
            y,
            w: node.w,
            h: node.h,
            corner_radius: 0.0,
        });
    }
    // A collapsed soft-wrapped run has no line to draw. Failing the whole
    // frame would hide the page that contains it, such as a closed dialog.
    let collapsed = text
        .max_width
        .get()
        .partial_cmp(&0.0)
        .is_none_or(|order| order != std::cmp::Ordering::Greater);
    let draws = !(text.style.soft_wrap && collapsed);
    commands.push(Command::Text {
        x: x + node.content_x,
        y: y + node.content_y,
        shaped: if draws {
            text.metrics
                .as_ref()
                .and_then(|metrics| metrics.shaped.clone())
        } else {
            None
        },
        color: text.color,
        span_paints: crate::text::span_paints(&text.spans),
    });
    for (rect, mark) in marks {
        commands.push(Command::Rect {
            x: x + rect.x,
            y: y + rect.y,
            w: rect.w,
            h: rect.h,
            corner_radius: 0.0,
            color: *mark,
            stroke_width: 0.0,
            stroke_color: [0, 0, 0, 0],
            gradient: None,
            shadow: None,
            box_shadows: Arc::from([]),
        });
    }
    if clip_overflow {
        commands.push(Command::Pop);
    }
}

pub(super) fn emit_image(commands: &mut Vec<Command>, node: &Node, x: f32, y: f32) {
    let Some(image) = node.image() else {
        return;
    };
    commands.push(Command::Image {
        x: x + node.content_x,
        y: y + node.content_y,
        w: if node.content_w > 0.0 {
            node.content_w
        } else {
            node.w
        },
        h: if node.content_h > 0.0 {
            node.content_h
        } else {
            node.h
        },
        source: Arc::clone(&image.source),
        mode: image.mode,
        required: image.required,
    });
}

pub(super) struct RecordFrame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    corner_radius: f32,
    alpha: Option<u8>,
    translate_x: f32,
    translate_y: f32,
    rotate_rad: f32,
    scale_x: f32,
    scale_y: f32,
    pushes_matrix: bool,
    pushes_opacity: bool,
    pushes_clip: bool,
    blend: Option<crate::scene::BlendMode>,
    blur: Option<f32>,
    backdrop: Option<f32>,
}

impl RecordFrame {
    pub(super) fn capture(node: &Node, x: f32, y: f32) -> Self {
        let geom = node.transform().placed(x, y, node.w, node.h);
        let alpha = node.alpha();
        let transformed = node.role.contains(Role::TRANSFORM);
        let clip = node.role.contains(Role::CLIP);
        let blend = node.blend();
        Self {
            x,
            y,
            w: node.w,
            h: node.h,
            corner_radius: if node.is_scroll() {
                0.0
            } else {
                node.corner_radius()
            },
            alpha,
            translate_x: geom.translate_x,
            translate_y: geom.translate_y,
            rotate_rad: geom.rotate_rad,
            scale_x: geom.scale_x,
            scale_y: geom.scale_y,
            pushes_matrix: transformed && !geom.is_identity(),
            pushes_opacity: alpha.is_some_and(|alpha| alpha != 255),
            pushes_clip: clip || node.is_scroll(),
            blend: (blend != crate::scene::BlendMode::SrcOver).then_some(blend),
            blur: positive_sigma(node.blur()),
            backdrop: positive_sigma(node.backdrop()),
        }
    }

    /// Transform, opacity, blend, backdrop, then blur. Content clip is separate.
    pub(super) fn push_outer(&self, commands: &mut Vec<Command>) {
        if self.pushes_matrix {
            commands.push(Command::PushTransform {
                translate_x: self.translate_x,
                translate_y: self.translate_y,
                rotate_rad: self.rotate_rad,
                scale_x: self.scale_x,
                scale_y: self.scale_y,
            });
        }
        if self.pushes_opacity {
            commands.push(Command::PushOpacity {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                alpha: self.alpha.unwrap_or(255),
            });
        }
        if let Some(mode) = self.blend {
            commands.push(Command::PushBlend {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                mode,
            });
        }
        if let Some(sigma) = self.backdrop {
            commands.push(Command::PushBackdropBlur {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                sigma,
            });
        }
        if let Some(sigma) = self.blur {
            commands.push(Command::PushBlur {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                sigma,
            });
        }
    }

    pub(super) fn push_content(&self, commands: &mut Vec<Command>) {
        if self.pushes_clip {
            commands.push(Command::PushClip {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                corner_radius: self.corner_radius,
            });
        }
    }

    pub(super) fn pop_content(&self, commands: &mut Vec<Command>) {
        if self.pushes_clip {
            commands.push(Command::Pop);
        }
    }

    pub(super) fn pop_outer(&self, commands: &mut Vec<Command>) {
        if self.blur.is_some() {
            commands.push(Command::Pop);
        }
        if self.backdrop.is_some() {
            commands.push(Command::Pop);
        }
        if self.blend.is_some() {
            commands.push(Command::Pop);
        }
        if self.pushes_opacity {
            commands.push(Command::Pop);
        }
        if self.pushes_matrix {
            commands.push(Command::Pop);
        }
    }
}
