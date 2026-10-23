use crate::{Kind, Scene};

/// Horizontal alignment of one paragraph. The default is the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Paragraph style carried on a text command. Defaults match the player before
/// these fields existed: one left-aligned run, weight 400, font line height.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// `None` uses the single family registered on the player.
    pub family: Option<String>,
    /// `None` leaves the line limit to the typography context.
    pub max_lines: Option<u32>,
    /// `None` leaves the ellipsis unset.
    pub ellipsis: Option<String>,
    pub align: TextAlign,
    /// Multiple of 100 from 100 through 900. 400 is regular.
    pub weight: u16,
    pub direction_rtl: bool,
    /// `0.0` uses the font's own line height.
    pub height: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: None,
            max_lines: None,
            ellipsis: None,
            align: TextAlign::Left,
            weight: 400,
            direction_rtl: false,
            height: 0.0,
        }
    }
}

/// One paint operation. Coordinates are absolute scene points.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Fill {
        color: [u8; 4],
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// Scene points. Zero is a sharp rectangle.
        corner_radius: f32,
        color: [u8; 4],
    },
    Text {
        x: f32,
        y: f32,
        /// Paragraph constraint used when this run was measured, in scene points.
        max_width: f32,
        font_size: f32,
        color: [u8; 4],
        utf8: String,
        style: TextStyle,
    },
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: String,
    },
    /// Later commands are clipped to this rounded rectangle until the matching [`Command::Pop`].
    PushClip {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    },
    /// Later commands are composited offscreen and drawn back at `alpha` until [`Command::Pop`].
    ///
    /// The box is the layer bounds. `alpha` is 0–255, the same range as a color channel.
    PushOpacity {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: u8,
    },
    /// Later commands are translated, rotated, then scaled until the matching [`Command::Pop`].
    ///
    /// `rotate_rad` is clockwise radians, the same direction as the player's rotate call
    /// after it converts to degrees. `scale` is uniform. The pivot is the current origin.
    /// Version 1 scene bytes never produce this command.
    PushTransform {
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale: f32,
    },
    /// Closes the most recent [`Command::PushClip`], [`Command::PushOpacity`], or
    /// [`Command::PushTransform`].
    Pop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub commands: Vec<Command>,
}

const V1_CLEAR: [u8; 4] = [17, 17, 17, 255];

/// Font size the version 1 byte format implies. The bytes themselves have no style fields.
pub const V1_FONT_SIZE: f32 = 28.0;

/// Paragraph width the version 1 byte format implies.
pub const V1_PARAGRAPH_WIDTH: f32 = 280.0;

/// Map a version 1 scene the way the old player painted it.
///
/// `LAYER` is skipped. The clear color is painted first. Text commands carry
/// [`V1_FONT_SIZE`] and [`V1_PARAGRAPH_WIDTH`] because the bytes have no style.
pub fn scene_to_draw_list(scene: &Scene) -> DrawList {
    let mut commands = vec![Command::Fill { color: V1_CLEAR }];
    for node in scene.nodes() {
        match node.kind {
            Kind::Layer => {}
            Kind::Rect => commands.push(Command::Rect {
                x: node.x,
                y: node.y,
                w: node.w,
                h: node.h,
                corner_radius: 0.0,
                color: node.color,
            }),
            Kind::Text => commands.push(Command::Text {
                x: node.x,
                y: node.y,
                max_width: V1_PARAGRAPH_WIDTH,
                font_size: V1_FONT_SIZE,
                color: node.color,
                utf8: node.text.clone(),
                style: TextStyle::default(),
            }),
            Kind::Image => commands.push(Command::Image {
                x: node.x,
                y: node.y,
                w: node.w,
                h: node.h,
                source: node.text.clone(),
            }),
        }
    }
    DrawList { commands }
}
