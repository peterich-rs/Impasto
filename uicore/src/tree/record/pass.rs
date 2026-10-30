//! Paint commands for one recorded node.

use std::sync::Arc;

use protocol::Command;

use super::super::node::{Kind, Node};
use super::{PreparedGradient, positive_sigma, prepared_gradient, rect_paint};

pub(super) fn emit_rect(commands: &mut Vec<Command>, node: &Node, x: f32, y: f32) {
    let (fill, stroke_width, stroke_color, gradient, shadow, box_shadows) = rect_paint(node);
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

pub(super) fn emit_text(
    text_scale: f32,
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
    let clip_overflow = text.style.overflow == protocol::TextOverflow::Clip
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
    let utf8 = if text.spans.is_empty() {
        Arc::clone(&text.utf8)
    } else {
        let joined: String = text.spans.iter().map(|span| span.utf8.as_str()).collect();
        debug_assert_eq!(joined.as_str(), text.utf8.as_ref());
        Arc::from(joined)
    };
    commands.push(Command::Text {
        x,
        y,
        max_width: text.max_width.get(),
        font_size: text.font_size * text_scale,
        paragraph: text.paragraph.unwrap_or(protocol::ParagraphId::NONE),
        color: text.color,
        utf8,
        style: Arc::clone(&text.style),
        spans: Arc::clone(&text.spans),
        links: Arc::clone(&text.links),
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
        x,
        y,
        w: node.w,
        h: node.h,
        source: Arc::clone(&image.source),
        mode: image.mode,
        required: image.required,
    });
}

pub(super) struct RecordFrame {
    kind: Kind,
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
    blend: Option<protocol::BlendMode>,
    blur: Option<f32>,
    backdrop: Option<f32>,
}

impl RecordFrame {
    pub(super) fn capture(node: &Node, x: f32, y: f32) -> Self {
        let geom = node.transform();
        let alpha = node.alpha();
        let kind = node.kind;
        let blend = node.blend();
        Self {
            kind,
            x,
            y,
            w: node.w,
            h: node.h,
            corner_radius: node.corner_radius(),
            alpha,
            translate_x: geom.translate_x,
            translate_y: geom.translate_y,
            rotate_rad: geom.rotate_rad,
            scale_x: geom.scale_x,
            scale_y: geom.scale_y,
            pushes_matrix: kind == Kind::Transform && !geom.is_identity(),
            pushes_opacity: kind == Kind::Opacity && alpha.unwrap_or(255) != 255,
            blend: (blend != protocol::BlendMode::SrcOver).then_some(blend),
            blur: positive_sigma(node.blur()),
            backdrop: positive_sigma(node.backdrop()),
        }
    }

    pub(super) fn push(&self, commands: &mut Vec<Command>) {
        match self.kind {
            Kind::Clip => commands.push(Command::PushClip {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                corner_radius: self.corner_radius,
            }),
            Kind::Opacity if self.pushes_opacity => commands.push(Command::PushOpacity {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                alpha: self.alpha.unwrap_or(255),
            }),
            Kind::Scroll => commands.push(Command::PushClip {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                corner_radius: 0.0,
            }),
            Kind::Transform if self.pushes_matrix => commands.push(Command::PushTransform {
                translate_x: self.translate_x,
                translate_y: self.translate_y,
                rotate_rad: self.rotate_rad,
                scale_x: self.scale_x,
                scale_y: self.scale_y,
            }),
            _ => {}
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
        if let Some(sigma) = self.blur {
            commands.push(Command::PushBlur {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
                sigma,
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
    }

    pub(super) fn pop(&self, commands: &mut Vec<Command>) {
        if self.backdrop.is_some() {
            commands.push(Command::Pop);
        }
        if self.blur.is_some() {
            commands.push(Command::Pop);
        }
        if self.blend.is_some() {
            commands.push(Command::Pop);
        }
        if matches!(self.kind, Kind::Clip | Kind::Scroll)
            || (self.kind == Kind::Opacity && self.pushes_opacity)
            || (self.kind == Kind::Transform && self.pushes_matrix)
        {
            commands.push(Command::Pop);
        }
    }
}
