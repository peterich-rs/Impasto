//! Text and geometry for the development error picture.

use std::sync::Arc;

use crate::geometry::Rect;
use crate::input::{Phase, PointerEvent};
use crate::scene::{Command, TextStyle};
use crate::text::{Shaper, metrics_key};

use super::Tree;

/// Origin of an error displayed in a debug application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevErrorCategory {
    Build,
    Linker,
    Native,
    HmrRejected,
    Runtime,
}

/// Error details owned by the UI session and recorded in the existing overlay picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevErrorOverlay {
    pub category: DevErrorCategory,
    pub title: String,
    pub message: String,
    pub location: Option<String>,
    pub stack: Vec<String>,
    pub snippet: Option<String>,
    pub collapsed: bool,
}

impl Tree {
    #[cfg(feature = "js")]
    pub(crate) fn set_dev_error(&mut self, mut overlay: Option<DevErrorOverlay>) -> bool {
        if let Some(next) = &mut overlay
            && let Some(current) = &self.debug.dev_error
        {
            next.collapsed = current.collapsed;
            if next != current {
                next.collapsed = false;
            }
        }
        if self.debug.dev_error == overlay {
            return false;
        }
        self.debug.dev_error = overlay;
        self.paint_layer_value();
        true
    }

    pub(super) fn dev_error_pointer(&mut self, event: PointerEvent) -> bool {
        if self.debug.dev_error_pointer == Some(event.pointer) {
            if matches!(event.phase, Phase::Up | Phase::Cancel) {
                self.debug.dev_error_pointer = None;
                if event.phase == Phase::Up
                    && let Some(overlay) = &mut self.debug.dev_error
                {
                    overlay.collapsed = !overlay.collapsed;
                    self.paint_layer_value();
                }
            }
            return true;
        }
        let Some(overlay) = &self.debug.dev_error else {
            return false;
        };
        if event.phase == Phase::Down
            && let Some((width, height)) = self.viewport
            && {
                let rect = button(panel(width, height, overlay.collapsed));
                (rect.x..=rect.x + rect.w).contains(&event.x)
                    && (rect.y..=rect.y + rect.h).contains(&event.y)
            }
        {
            self.debug.dev_error_pointer = Some(event.pointer);
            return true;
        }
        false
    }

    pub(super) fn record_dev_error(&mut self, commands: &mut Vec<Command>) {
        if let (Some(overlay), Some((width, height))) = (&self.debug.dev_error, self.viewport) {
            overlay.record(
                &mut *self.shaper,
                panel(width, height, overlay.collapsed),
                commands,
            );
        }
    }
}

impl DevErrorOverlay {
    fn record(&self, shaper: &mut dyn Shaper, rect: Rect, commands: &mut Vec<Command>) {
        if rect.w < 92.0 || rect.h < 32.0 {
            return;
        }
        let button = button(rect);
        commands.push(box_command(rect, [36, 19, 24, 255], [219, 81, 92, 255]));
        commands.push(box_command(button, [89, 38, 47, 255], [130, 62, 71, 255]));
        draw(
            shaper,
            if self.collapsed { "+" } else { "−" },
            button.x + 7.0,
            button.y + 2.0,
            button.w - 10.0,
            16.0,
            [255; 4],
            commands,
        );
        draw(
            shaper,
            &self.title,
            rect.x + 12.0,
            rect.y + 9.0,
            rect.w - 56.0,
            15.0,
            [255, 233, 235, 255],
            commands,
        );
        if self.collapsed {
            return;
        }
        let mut y = rect.y + 38.0;
        let width = rect.w - 24.0;
        let bottom = rect.y + rect.h - 12.0;
        let line = self
            .location
            .iter()
            .map(String::as_str)
            .chain(self.message.lines())
            .chain(self.snippet.iter().flat_map(|snippet| snippet.lines()))
            .chain(self.stack.iter().map(String::as_str));
        for text in line.take(24) {
            if y + 18.0 > bottom {
                break;
            }
            y += draw(
                shaper,
                text,
                rect.x + 12.0,
                y,
                width,
                13.0,
                [241, 207, 212, 255],
                commands,
            ) + 2.0;
        }
    }
}

fn panel(width: f32, height: f32, collapsed: bool) -> Rect {
    let width = (width - 24.0).clamp(0.0, 720.0);
    let height = if collapsed {
        38.0
    } else {
        (height - 28.0).clamp(38.0, 340.0)
    };
    Rect {
        x: 12.0,
        y: 14.0,
        w: width,
        h: height,
    }
}

fn button(panel: Rect) -> Rect {
    Rect {
        x: panel.x + panel.w - 32.0,
        y: panel.y + 7.0,
        w: 24.0,
        h: 24.0,
    }
}

fn box_command(rect: Rect, color: [u8; 4], stroke_color: [u8; 4]) -> Command {
    Command::Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        corner_radius: 5.0,
        color,
        stroke_width: 1.0,
        stroke_color,
        gradient: None,
        shadow: None,
        box_shadows: Default::default(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "drawing one text line requires its geometry and paint"
)]
fn draw(
    shaper: &mut dyn Shaper,
    text: &str,
    x: f32,
    y: f32,
    width: f32,
    size: f32,
    color: [u8; 4],
    commands: &mut Vec<Command>,
) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the panel uses a bounded estimate of whole text characters"
    )]
    let max_chars = (width / (size * 0.55)) as usize;
    let mut text = text.chars().take(max_chars).collect::<String>();
    if text.is_empty() {
        text.push(' ');
    }
    let style = Arc::new(TextStyle::default());
    let key = metrics_key(Arc::from(text), style, &[], size, width);
    let metrics = shaper.metrics(&key);
    commands.push(Command::Text {
        x,
        y,
        shaped: metrics.shaped.clone(),
        color,
        span_paints: Default::default(),
    });
    metrics.size.height.max(size)
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use crate::text::MonoShaper;

    #[test]
    fn clears_the_error_picture_from_the_same_tree() {
        let mut tree = Tree::new(MonoShaper::default());
        tree.text("stable".to_owned(), 16., [255; 4]).unwrap();
        tree.set_viewport(0., 0., 400., 320.).unwrap();
        tree.layout().unwrap();
        tree.record_layers().unwrap();
        let picture_count = tree.layers.pictures.len();
        let error = DevErrorOverlay {
            category: DevErrorCategory::Build,
            title: "Could not compile".into(),
            message: "Unexpected token".into(),
            location: Some("src/main.tsx:1:1".into()),
            stack: Vec::new(),
            snippet: None,
            collapsed: false,
        };
        assert!(tree.set_dev_error(Some(error.clone())));
        assert!(!tree.set_dev_error(Some(error)));
        tree.record_layers().unwrap();
        assert_eq!(tree.layers.pictures.len(), picture_count + 1);
        assert_eq!(
            tree.layers.pictures.last().unwrap().id,
            super::super::repaint::OVERLAY_PICTURE_ID
        );
        assert!(tree.set_dev_error(None));
        tree.record_layers().unwrap();
        assert_eq!(tree.layers.pictures.len(), picture_count);
    }
}
