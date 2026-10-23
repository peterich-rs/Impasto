//! Record a [`DrawList`] from the laid-out tree.

use protocol::{Command, DrawList};

use super::node::Kind;
use super::{NodeId, Tree};
use crate::Error;

impl Tree {
    pub fn record(&self) -> Result<DrawList, Error> {
        let mut commands = Vec::new();
        self.record_node(self.root.ok_or(Error::NoRoot)?, &mut commands, (0.0, 0.0));
        Ok(DrawList { commands })
    }

    pub(super) fn record_node(&self, id: NodeId, commands: &mut Vec<Command>, offset: (f32, f32)) {
        let Some(node) = self.get(id) else {
            return;
        };
        let x = node.x - offset.0;
        let y = node.y - offset.1;
        if let Some(color) = node.fill
            && node.w > 0.0
            && node.h > 0.0
        {
            commands.push(Command::Rect {
                x,
                y,
                w: node.w,
                h: node.h,
                corner_radius: node.corner_radius,
                color,
            });
        }
        if let Some(text) = &node.text
            && !text.utf8.is_empty()
        {
            commands.push(Command::Text {
                x,
                y,
                max_width: text.max_width.get(),
                font_size: text.font_size,
                color: text.color,
                utf8: text.utf8.clone(),
                style: text.style.clone(),
            });
        }
        if let Some(image) = &node.image {
            commands.push(Command::Image {
                x,
                y,
                w: node.w,
                h: node.h,
                source: image.source.clone(),
            });
        }
        let kind = node.kind;
        let (w, h) = (node.w, node.h);
        let corner_radius = node.corner_radius;
        let alpha = node.alpha;
        let scroll = node.scroll;
        let transform = (
            node.translate_x,
            node.translate_y,
            node.rotate_rad,
            node.scale,
        );
        let child_offset = if kind == Kind::Scroll {
            (offset.0 + scroll.0, offset.1 + scroll.1)
        } else {
            offset
        };
        match kind {
            Kind::Clip => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius,
            }),
            Kind::Opacity => commands.push(Command::PushOpacity {
                x,
                y,
                w,
                h,
                alpha: alpha.unwrap_or(255),
            }),
            Kind::Scroll => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius: 0.0,
            }),
            Kind::Transform => commands.push(Command::PushTransform {
                translate_x: transform.0,
                translate_y: transform.1,
                rotate_rad: transform.2,
                scale: transform.3,
            }),
            _ => {}
        }
        for child in &node.children {
            self.record_node(*child, commands, child_offset);
        }
        if matches!(
            kind,
            Kind::Clip | Kind::Opacity | Kind::Scroll | Kind::Transform
        ) {
            commands.push(Command::Pop);
        }
    }
}
