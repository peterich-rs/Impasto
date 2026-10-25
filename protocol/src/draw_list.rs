use std::sync::Arc;

use crate::{Kind, Scene};

/// Horizontal alignment of one paragraph. The default is the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// What happens to text that does not fit the paragraph box.
///
/// `Clip` drops the extra lines. `Ellipsis` replaces the tail of the last
/// visible line. `Visible` keeps every line, and a line may be wider than the
/// constraint when soft wrap is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
    Visible,
}

/// Paragraph style carried on a text command. Defaults match the player before
/// these fields existed: one left-aligned run, weight 400, font line height.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// `None` uses the single family registered on the player.
    pub family: Option<String>,
    /// `None` leaves the line limit to the typography context.
    ///
    /// Ignored when [`TextOverflow::Visible`] is set.
    pub max_lines: Option<u32>,
    /// Glyphs that replace the tail. Used when [`TextOverflow::Ellipsis`] is set.
    /// `None` means the default ellipsis `…`.
    pub ellipsis: Option<String>,
    pub align: TextAlign,
    /// Multiple of 100 from 100 through 900. 400 is regular.
    pub weight: u16,
    pub direction_rtl: bool,
    /// `0.0` uses the font's own line height. A positive value is a multiple of
    /// the font size.
    pub height: f32,
    /// Draw an underline with the run's foreground.
    pub underline: bool,
    /// Draw a line through the run with the run's foreground.
    pub line_through: bool,
    /// Break a line that is wider than the constraint. Hard breaks still apply
    /// when this is false.
    pub soft_wrap: bool,
    pub overflow: TextOverflow,
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
            underline: false,
            line_through: false,
            soft_wrap: true,
            overflow: TextOverflow::Clip,
        }
    }
}

impl TextStyle {
    /// Ellipsis glyphs, when this paragraph should truncate.
    pub fn ellipsis_str(&self) -> Option<&str> {
        if self.overflow != TextOverflow::Ellipsis {
            return None;
        }
        Some(
            self.ellipsis
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or("…"),
        )
    }
}

/// One styled run. A text command with spans uses their concatenation as `utf8`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub utf8: String,
    pub color: [u8; 4],
    pub weight: u16,
    pub underline: bool,
    pub line_through: bool,
    pub family: Option<String>,
}

/// Inclusive-exclusive UTF-16 range. `id` is the value delivered with `ActivateLink`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextLink {
    pub start: u32,
    pub end: u32,
    pub id: u32,
}

/// How an image is fitted into its layout box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ContentMode {
    #[default]
    Fill,
    Fit,
    Crop,
}

/// One color stop on a gradient. `offset` is in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f32,
    pub color: [u8; 4],
}

/// Fill gradient in the rectangle's unit square. `(0, 0)` is the top-left.
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        stops: Vec<ColorStop>,
    },
    Radial {
        center: [f32; 2],
        radius: f32,
        stops: Vec<ColorStop>,
    },
}

/// Material shadow. `elevation` is scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: [u8; 4],
    pub elevation: f32,
}

/// Layer blend. [`BlendMode::SrcOver`] is a no-op at record time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    SrcOver,
    Multiply,
    Screen,
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
        /// Scene points. Zero, negative, or non-finite means no stroke.
        stroke_width: f32,
        stroke_color: [u8; 4],
        gradient: Option<Gradient>,
        shadow: Option<Shadow>,
    },
    Text {
        x: f32,
        y: f32,
        /// Paragraph constraint used when this run was measured, in scene points.
        max_width: f32,
        font_size: f32,
        color: [u8; 4],
        utf8: Arc<str>,
        style: TextStyle,
        /// Empty means one run with the command color and style.
        spans: Vec<TextSpan>,
        /// UTF-16 ranges. Interaction only; the player does not draw them.
        links: Vec<TextLink>,
    },
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: Arc<str>,
        mode: ContentMode,
        /// Missing required images fail the frame. Optional images are skipped.
        required: bool,
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
    /// Later commands are scaled, rotated clockwise, then translated until [`Command::Pop`].
    ///
    /// `rotate_rad` is clockwise radians. The player still calls Translate, Rotate
    /// (degrees), Scale in that order. Version 1 scene bytes never produce this command.
    PushTransform {
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    },
    /// Blur the layer contents. `sigma` is scene points. Non-positive or non-finite
    /// values are ignored: no push and no matching pop.
    PushBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    /// Blur the backdrop behind this layer. `sigma` is scene points.
    PushBackdropBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    /// Composite this layer with Multiply or Screen. [`BlendMode::SrcOver`] is a no-op.
    PushBlend {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    },
    /// Closes the most recent push. Invalid blur and SrcOver blend do not push,
    /// so they also do not consume a pop at record time.
    Pop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub commands: Vec<Command>,
}

impl DrawList {
    /// Image sources in paint order. The version 1 byte format is unchanged.
    pub fn image_sources(&self) -> impl Iterator<Item = &str> {
        self.commands.iter().filter_map(|command| match command {
            Command::Image { source, .. } => Some(source.as_ref()),
            _ => None,
        })
    }

    /// Unique image sources. A source that is both required and optional is required.
    pub fn image_requirements(&self) -> Vec<(Arc<str>, bool)> {
        let mut out: Vec<(Arc<str>, bool)> = Vec::new();
        for command in &self.commands {
            let Command::Image {
                source, required, ..
            } = command
            else {
                continue;
            };
            if let Some((_, was)) = out.iter_mut().find(|(name, _)| name == source) {
                *was |= *required;
            } else {
                out.push((Arc::clone(source), *required));
            }
        }
        out
    }
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
                stroke_width: 0.0,
                stroke_color: [0, 0, 0, 0],
                gradient: None,
                shadow: None,
            }),
            Kind::Text => commands.push(Command::Text {
                x: node.x,
                y: node.y,
                max_width: V1_PARAGRAPH_WIDTH,
                font_size: V1_FONT_SIZE,
                color: node.color,
                utf8: Arc::from(node.text.as_str()),
                style: TextStyle::default(),
                spans: Vec::new(),
                links: Vec::new(),
            }),
            Kind::Image => commands.push(Command::Image {
                x: node.x,
                y: node.y,
                w: node.w,
                h: node.h,
                source: Arc::from(node.text.as_str()),
                mode: ContentMode::Fill,
                required: true,
            }),
        }
    }
    DrawList { commands }
}
