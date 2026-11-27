//! Recording and replay of the flat command stream.

use std::sync::Arc;

use super::super::node::NodeFlags;
use super::pass::{self, RecordFrame, emit_image, emit_rect, emit_text};
use super::{FlatList, NodeId, PaintClip, RecordAt, Tree, intersect, outside, paint_kind};
use crate::scene::{Command, ScrollTargetProp};

impl Tree {
    pub(super) fn record_node(
        &mut self,
        id: NodeId,
        commands: &mut Vec<Command>,
        at: RecordAt,
    ) -> usize {
        let marks = self.editor_mark_rects(id);
        let Some(node) = self.get(id) else {
            return 0;
        };
        if node.flags.contains(NodeFlags::CULL_PAINT)
            || node.flags.contains(NodeFlags::HIDDEN)
            || node.alpha() == Some(0)
                && !self
                    .motion_layers
                    .get(&id)
                    .is_some_and(|layers| layers.transition_opacity)
        {
            return 1;
        }
        let abs_x = at.parent.0 + node.x;
        let abs_y = at.parent.1 + node.y;
        let x = abs_x - at.offset.0 - at.space.0;
        let y = abs_y - at.offset.1 - at.space.1;
        let paint = paint_kind(node);
        if !paint.transform && outside(at.clip, x, y, node.w, node.h) {
            return 1;
        }
        let frame = RecordFrame::capture(node, x, y);
        if at.descend {
            frame.push_outer(commands);
        }
        let replace_background = self.scroll_bindings.get(&id).is_some_and(|rules| {
            rules
                .values()
                .any(|rule| rule.prop == ScrollTargetProp::BackgroundColor)
        });
        if at.descend || !self.uses_paint_slot(id) {
            emit_rect(commands, node, x, y, replace_background);
        }
        pass::emit_outline(commands, node, x, y);
        if at.descend {
            frame.push_content(commands);
        }
        if node.content_w > 0.0
            && node.content_h > 0.0
            && let Some(list) = node.canvas.clone()
        {
            commands.push(Command::Canvas {
                x: x + node.content_x,
                y: y + node.content_y,
                width: node.content_w,
                height: node.content_h,
                list,
            });
        }
        if node.content_w > 0.0
            && node.content_h > 0.0
            && let Some(effect) = self.shader_effects.get(&id)
        {
            commands.push(Command::ShaderEffect {
                bounds: [
                    x + node.content_x,
                    y + node.content_y,
                    node.content_w,
                    node.content_h,
                ],
                shader: effect.shader,
                uniforms: Arc::clone(&effect.uniforms),
                textures: Arc::clone(&effect.textures),
            });
        }
        emit_text(commands, node, x, y, &marks);
        emit_image(commands, node, x, y);
        if !at.descend {
            return 1;
        }
        let scroll = node.scroll_paint();
        let child_offset = if paint.scroll {
            (at.offset.0 + scroll.0, at.offset.1 + scroll.1)
        } else {
            at.offset
        };
        let child_clip = if paint.scroll || paint.clip {
            intersect(at.clip, x, y, node.w, node.h)
        } else if paint.transform {
            None
        } else {
            at.clip
        };
        let first = node.first_child;
        let ordered = self.ordered_children(id, false);
        let mut visits = 1;
        if let Some(list) = ordered {
            for current in list {
                visits += self.record_node(
                    current,
                    commands,
                    RecordAt {
                        parent: (abs_x, abs_y),
                        offset: child_offset,
                        space: at.space,
                        clip: child_clip,
                        descend: true,
                    },
                );
            }
        } else {
            let mut child = first;
            while let Some(current) = child {
                let next = self.get(current).and_then(|node| node.next);
                visits += self.record_node(
                    current,
                    commands,
                    RecordAt {
                        parent: (abs_x, abs_y),
                        offset: child_offset,
                        space: at.space,
                        clip: child_clip,
                        descend: true,
                    },
                );
                child = next;
            }
        }
        frame.pop_content(commands);
        frame.pop_outer(commands);
        visits
    }

    pub(super) fn blit(
        &self,
        id: NodeId,
        out: &mut FlatList<'_>,
        dx: f32,
        dy: f32,
        clip: Option<PaintClip>,
    ) {
        let (Some(out), Some(picture)) = (out.0.as_deref_mut(), self.pictures.get(&id)) else {
            return;
        };
        self.replay_commands(picture.commands().as_ref(), dx, dy, clip, out);
    }

    pub(super) fn replay_commands(
        &self,
        commands: &[Command],
        dx: f32,
        dy: f32,
        clip: Option<PaintClip>,
        out: &mut Vec<Command>,
    ) {
        for command in commands {
            let moved = translate_command(command, dx, dy);
            if is_leaf_paint(&moved) && command_outside(&moved, clip) {
                continue;
            }
            out.push(moved);
        }
    }
}

pub(super) fn translate_command(command: &Command, dx: f32, dy: f32) -> Command {
    if dx == 0.0 && dy == 0.0 {
        return command.clone();
    }
    match command.clone() {
        Command::Rect {
            x,
            y,
            w,
            h,
            corner_radius,
            color,
            stroke_width,
            stroke_color,
            gradient,
            shadow,
            box_shadows,
        } => Command::Rect {
            x: x + dx,
            y: y + dy,
            w,
            h,
            corner_radius,
            color,
            stroke_width,
            stroke_color,
            gradient,
            shadow,
            box_shadows,
        },
        Command::ShapedRect {
            mut bounds,
            corners,
            paint,
        } => {
            bounds[0] += dx;
            bounds[1] += dy;
            Command::ShapedRect {
                bounds,
                corners,
                paint,
            }
        }
        Command::PushCornerClip {
            mut bounds,
            corners,
        } => {
            bounds[0] += dx;
            bounds[1] += dy;
            Command::PushCornerClip { bounds, corners }
        }
        Command::Text {
            x,
            y,
            shaped,
            color,
            span_paints,
        } => Command::Text {
            x: x + dx,
            y: y + dy,
            shaped,
            color,
            span_paints,
        },
        Command::Image {
            x,
            y,
            w,
            h,
            source,
            mode,
            required,
        } => Command::Image {
            x: x + dx,
            y: y + dy,
            w,
            h,
            source,
            mode,
            required,
        },
        Command::PushClip {
            x,
            y,
            w,
            h,
            corner_radius,
        } => Command::PushClip {
            x: x + dx,
            y: y + dy,
            w,
            h,
            corner_radius,
        },
        Command::PushOpacity { x, y, w, h, alpha } => Command::PushOpacity {
            x: x + dx,
            y: y + dy,
            w,
            h,
            alpha,
        },
        Command::PushBlur { x, y, w, h, sigma } => Command::PushBlur {
            x: x + dx,
            y: y + dy,
            w,
            h,
            sigma,
        },
        Command::PushBackdropMaterial {
            bounds: [x, y, w, h],
            corners,
            material,
        } => Command::PushBackdropMaterial {
            bounds: [x + dx, y + dy, w, h],
            corners,
            material,
        },
        Command::PushBackdropBlur { x, y, w, h, sigma } => Command::PushBackdropBlur {
            x: x + dx,
            y: y + dy,
            w,
            h,
            sigma,
        },
        Command::PushBlend { x, y, w, h, mode } => Command::PushBlend {
            x: x + dx,
            y: y + dy,
            w,
            h,
            mode,
        },
        Command::Canvas {
            x,
            y,
            width,
            height,
            list,
        } => Command::Canvas {
            x: x + dx,
            y: y + dy,
            width,
            height,
            list,
        },
        Command::ShaderEffect {
            mut bounds,
            shader,
            uniforms,
            textures,
        } => {
            bounds[0] += dx;
            bounds[1] += dy;
            Command::ShaderEffect {
                bounds,
                shader,
                uniforms,
                textures,
            }
        }
        other => other,
    }
}

fn is_leaf_paint(command: &Command) -> bool {
    matches!(
        command,
        Command::Rect { .. }
            | Command::ShapedRect { .. }
            | Command::Text { .. }
            | Command::Image { .. }
            | Command::Canvas { .. }
            | Command::ShaderEffect { .. }
    )
}

fn command_outside(command: &Command, clip: Option<PaintClip>) -> bool {
    match command {
        Command::ShapedRect {
            bounds: [x, y, w, h],
            ..
        }
        | Command::ShaderEffect {
            bounds: [x, y, w, h],
            ..
        }
        | Command::PushCornerClip {
            bounds: [x, y, w, h],
            ..
        }
        | Command::PushBackdropMaterial {
            bounds: [x, y, w, h],
            ..
        } => outside(clip, *x, *y, *w, *h),
        Command::Rect { x, y, w, h, .. }
        | Command::Image { x, y, w, h, .. }
        | Command::PushClip { x, y, w, h, .. }
        | Command::PushOpacity { x, y, w, h, .. }
        | Command::PushBlur { x, y, w, h, .. }
        | Command::PushBackdropBlur { x, y, w, h, .. }
        | Command::PushBlend { x, y, w, h, .. } => outside(clip, *x, *y, *w, *h),
        Command::Text { x, y, shaped, .. } => {
            // Cull on the frozen glyph extent. A paragraph without glyphs
            // paints nothing, so it is always outside the clip.
            shaped.as_deref().is_none_or(|shaped| {
                outside(
                    clip,
                    *x,
                    *y,
                    shaped.size[0].max(1.0),
                    shaped.size[1].max(1.0),
                )
            })
        }
        Command::Canvas {
            x,
            y,
            width,
            height,
            ..
        } => outside(clip, *x, *y, *width, *height),
        Command::Fill { .. }
        | Command::PushTransform { .. }
        | Command::PushColorFilter { .. }
        | Command::Pop => false,
    }
}
