use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::paint::{ColorStop, Gradient};

/// Identity of one font face inside a window's shared font library.
///
/// The ID is pure data so a shaped paragraph can cross to the raster queue.
/// Glyph IDs are resolved against the exact face file the library stored
/// under this ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontFaceId(pub u64);

/// Glyphs of one laid-out paragraph, frozen after the final line break.
///
/// Coordinates are scene points relative to the paragraph's top-left, Y
/// down. They already include visual order, line alignment, baseline and
/// glyph offsets, so the raster only places and samples them.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedParagraph {
    pub size: [f32; 2],
    pub runs: Box<[ShapedRun]>,
    pub decorations: Box<[DecorationLine]>,
}

impl ShapedParagraph {
    /// False when there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|run| run.glyphs.is_empty())
    }
}

/// One run of glyphs that share a face and size.
///
/// `span_index` is the logical span whose brush paints this run. `None`
/// uses the command's default color. `span_bounds` is that span's box in
/// paragraph space, used for gradient coordinates when a fallback font
/// splits one span into several runs on the same line.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedRun {
    pub face: Arc<FontFaceId>,
    pub font_size: f32,
    /// Normalized variation coordinates, in the face's axis order.
    pub normalized_coords: Box<[i16]>,
    /// Synthetic bold requested by the shaper.
    pub embolden: bool,
    /// Synthetic oblique angle, clockwise degrees.
    pub skew_degrees: Option<f32>,
    pub span_index: Option<u32>,
    pub span_bounds: [f32; 4],
    pub glyphs: Box<[PositionedGlyph]>,
}

/// One glyph at its final position, baseline-relative to the paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

/// An underline or strike-through bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationLine {
    /// `[x, y, width, height]` in paragraph space.
    pub rect: [f32; 4],
    pub span_index: Option<u32>,
    pub span_bounds: [f32; 4],
}

/// Horizontal alignment of one paragraph. The default is the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Upright or italic. Oblique angles are not a separate style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FontSlant {
    #[default]
    Normal,
    Italic,
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
    /// Default is upright. Italic is part of the measurement key.
    pub slant: FontSlant,
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
            slant: FontSlant::Normal,
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
            && self.slant == other.slant
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
        self.slant.hash(state);
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
