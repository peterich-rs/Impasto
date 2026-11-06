use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Slot and generation of a retained paragraph.
///
/// `generation` is 0 when the command has no live paragraph. The player then
/// rebuilds from the payload and does not touch a released slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParagraphId {
    pub index: u32,
    pub generation: u32,
}

impl ParagraphId {
    /// Rebuild from the command payload. Never matches a retained slot.
    pub const NONE: Self = Self {
        index: u32::MAX,
        generation: 0,
    };

    pub const fn is_none(self) -> bool {
        self.generation == 0
    }
}

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
#[derive(Clone, Debug)]
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

impl PartialEq for TextStyle {
    fn eq(&self, other: &Self) -> bool {
        self.family == other.family
            && self.max_lines == other.max_lines
            && self.ellipsis == other.ellipsis
            && self.align == other.align
            && self.weight == other.weight
            && self.direction_rtl == other.direction_rtl
            && self.height.to_bits() == other.height.to_bits()
            && self.underline == other.underline
            && self.line_through == other.line_through
            && self.soft_wrap == other.soft_wrap
            && self.overflow == other.overflow
    }
}

impl Eq for TextStyle {}

impl Hash for TextStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.family.hash(state);
        self.max_lines.hash(state);
        self.ellipsis.hash(state);
        self.align.hash(state);
        self.weight.hash(state);
        self.direction_rtl.hash(state);
        self.height.to_bits().hash(state);
        self.underline.hash(state);
        self.line_through.hash(state);
        self.soft_wrap.hash(state);
        self.overflow.hash(state);
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
///
/// `gradient` replaces `color` as the foreground. Underline and strike stay on `color`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub utf8: String,
    pub color: [u8; 4],
    pub gradient: Option<Gradient>,
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
///
/// [`ContentMode::NineSlice`] insets are source pixels. The player slices the
/// decoded image; record still emits one image command.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ContentMode {
    #[default]
    Fill,
    Fit,
    Crop,
    NineSlice {
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
}

/// Color space of one gradient stop. Only sRGB is defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ColorSpace {
    #[default]
    Srgb,
}

/// One color stop on a gradient. `offset` is in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f32,
    pub color: [u8; 4],
    pub space: ColorSpace,
}

impl ColorStop {
    pub const fn srgb(offset: f32, color: [u8; 4]) -> Self {
        Self {
            offset,
            color,
            space: ColorSpace::Srgb,
        }
    }
}

/// Fill gradient in the rectangle's unit square. `(0, 0)` is the top-left.
/// Sweep angles are radians, clockwise from the positive x axis.
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
    Sweep {
        center: [f32; 2],
        start_angle_rad: f32,
        end_angle_rad: f32,
        stops: Vec<ColorStop>,
    },
}

/// Foreground of one span. `Eq` and `Hash` fold `-0.0` into `0.0`.
#[derive(Clone, Debug)]
pub enum SpanPaint {
    Color([u8; 4]),
    Gradient(Gradient),
}

impl TextSpan {
    /// Foreground used for caching and drawing. A gradient wins over `color`.
    pub fn paint(&self) -> SpanPaint {
        match &self.gradient {
            Some(gradient) => SpanPaint::Gradient(gradient.clone()),
            None => SpanPaint::Color(self.color),
        }
    }
}

fn num_bits(value: f32) -> u32 {
    let bits = value.to_bits();
    if bits == (-0.0f32).to_bits() { 0 } else { bits }
}

fn pair_eq(left: [f32; 2], right: [f32; 2]) -> bool {
    num_bits(left[0]) == num_bits(right[0]) && num_bits(left[1]) == num_bits(right[1])
}

fn stops_eq(left: &[ColorStop], right: &[ColorStop]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            num_bits(a.offset) == num_bits(b.offset) && a.color == b.color && a.space == b.space
        })
}

fn gradient_eq(left: &Gradient, right: &Gradient) -> bool {
    match (left, right) {
        (
            Gradient::Linear {
                start: a,
                end: b,
                stops: s,
            },
            Gradient::Linear {
                start: c,
                end: d,
                stops: t,
            },
        ) => pair_eq(*a, *c) && pair_eq(*b, *d) && stops_eq(s, t),
        (
            Gradient::Radial {
                center: a,
                radius: b,
                stops: s,
            },
            Gradient::Radial {
                center: c,
                radius: d,
                stops: t,
            },
        ) => pair_eq(*a, *c) && num_bits(*b) == num_bits(*d) && stops_eq(s, t),
        (
            Gradient::Sweep {
                center: a,
                start_angle_rad: b0,
                end_angle_rad: b1,
                stops: s,
            },
            Gradient::Sweep {
                center: c,
                start_angle_rad: d0,
                end_angle_rad: d1,
                stops: t,
            },
        ) => {
            pair_eq(*a, *c)
                && num_bits(*b0) == num_bits(*d0)
                && num_bits(*b1) == num_bits(*d1)
                && stops_eq(s, t)
        }
        _ => false,
    }
}

fn hash_num<H: Hasher>(value: f32, state: &mut H) {
    num_bits(value).hash(state);
}

fn hash_pair<H: Hasher>(value: [f32; 2], state: &mut H) {
    hash_num(value[0], state);
    hash_num(value[1], state);
}

fn hash_stops<H: Hasher>(stops: &[ColorStop], state: &mut H) {
    stops.len().hash(state);
    for stop in stops {
        hash_num(stop.offset, state);
        stop.color.hash(state);
        stop.space.hash(state);
    }
}

fn hash_gradient<H: Hasher>(gradient: &Gradient, state: &mut H) {
    match gradient {
        Gradient::Linear { start, end, stops } => {
            1u8.hash(state);
            hash_pair(*start, state);
            hash_pair(*end, state);
            hash_stops(stops, state);
        }
        Gradient::Radial {
            center,
            radius,
            stops,
        } => {
            2u8.hash(state);
            hash_pair(*center, state);
            hash_num(*radius, state);
            hash_stops(stops, state);
        }
        Gradient::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
            stops,
        } => {
            3u8.hash(state);
            hash_pair(*center, state);
            hash_num(*start_angle_rad, state);
            hash_num(*end_angle_rad, state);
            hash_stops(stops, state);
        }
    }
}

impl PartialEq for SpanPaint {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Color(left), Self::Color(right)) => left == right,
            (Self::Gradient(left), Self::Gradient(right)) => gradient_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for SpanPaint {}

impl Hash for SpanPaint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Color(color) => {
                0u8.hash(state);
                color.hash(state);
            }
            Self::Gradient(gradient) => {
                1u8.hash(state);
                hash_gradient(gradient, state);
            }
        }
    }
}

/// Material shadow. `elevation` is scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: [u8; 4],
    pub elevation: f32,
}

/// A drop shadow drawn in the same picture as its rectangle.
///
/// `offset` and `blur_sigma` are scene points. The player does not multiply
/// `blur_sigma` by `point_scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset: [f32; 2],
    pub blur_sigma: f32,
    pub spread: f32,
    pub color: [u8; 4],
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
        /// Drawn before the rectangle, in array order. Empty means no box shadow.
        box_shadows: Arc<[BoxShadow]>,
    },
    Text {
        x: f32,
        y: f32,
        /// Paragraph constraint used when this run was measured, in scene points.
        max_width: f32,
        font_size: f32,
        /// Retained paragraph. [`ParagraphId::NONE`] rebuilds from the payload.
        paragraph: ParagraphId,
        color: [u8; 4],
        utf8: Arc<str>,
        style: Arc<TextStyle>,
        /// Empty means one run with the command color and style.
        spans: Arc<[TextSpan]>,
        /// UTF-16 ranges. Interaction only; the player does not draw them.
        links: Arc<[TextLink]>,
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
    /// Image sources in paint order.
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

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::{ColorStop, Command, DrawList, Gradient, SpanPaint, TextSpan};

    #[test]
    fn an_unbalanced_pop_is_stored_as_data() {
        let list = DrawList {
            commands: vec![Command::Pop],
        };
        assert_eq!(list.commands, vec![Command::Pop]);
    }

    fn bits(paint: &SpanPaint) -> u64 {
        let mut hasher = DefaultHasher::new();
        paint.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn a_gradient_span_hashes_negative_zero_with_zero() {
        let stops = vec![
            ColorStop::srgb(0.0, [0, 0, 0, 255]),
            ColorStop::srgb(1.0, [255, 0, 0, 255]),
        ];
        let left = TextSpan {
            utf8: "A".into(),
            color: [1, 2, 3, 4],
            gradient: Some(Gradient::Linear {
                start: [-0.0, 0.0],
                end: [1.0, 0.0],
                stops: stops.clone(),
            }),
            weight: 400,
            underline: false,
            line_through: false,
            family: None,
        };
        let right = TextSpan {
            gradient: Some(Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, -0.0],
                stops,
            }),
            ..left.clone()
        };
        assert_eq!(left.paint(), right.paint());
        assert_eq!(bits(&left.paint()), bits(&right.paint()));
        let solid = TextSpan {
            gradient: None,
            ..left
        };
        assert_ne!(solid.paint(), right.paint());
    }
}
