//! Lengths, tracks, and other style values that are not a single number.
//!
//! A class `px` is one scene point, not a device pixel. Percentages stay on
//! the parent until Taffy resolves them. `rem`, viewport, spacing steps, and
//! tokens stay unresolved until the environment is applied.

use std::sync::Arc;

/// Collapse `-0.0` so equal lengths compare equal after a write.
pub fn canonical(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

pub fn is_finite(value: f32) -> bool {
    value.is_finite()
}

/// Theme length name. The environment owns the numeric value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LengthToken(Arc<str>);

impl LengthToken {
    pub fn new(name: &str) -> Self {
        Self(Arc::from(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A length that may still depend on the environment.
///
/// `Fraction(1.0)` is 100% of the parent on the axis Taffy resolves.
/// `Steps(4)` is four spacing units (`p-4`), not 4 points.
#[derive(Clone, Debug, PartialEq)]
pub enum Length {
    Points(f32),
    Fraction(f32),
    Rem(f32),
    ViewportWidth(f32),
    ViewportHeight(f32),
    Steps(f32),
    Token(LengthToken),
}

impl Length {
    pub fn points(value: f32) -> Self {
        Self::Points(canonical(value))
    }

    pub fn fraction(value: f32) -> Self {
        Self::Fraction(canonical(value))
    }

    pub fn rem(value: f32) -> Self {
        Self::Rem(canonical(value))
    }

    pub fn steps(value: f32) -> Self {
        Self::Steps(canonical(value))
    }

    pub fn token(name: &str) -> Self {
        Self::Token(LengthToken::new(name))
    }

    pub fn finite_number(&self) -> Option<f32> {
        match self {
            Self::Points(value)
            | Self::Fraction(value)
            | Self::Rem(value)
            | Self::ViewportWidth(value)
            | Self::ViewportHeight(value)
            | Self::Steps(value) => is_finite(*value).then_some(*value),
            Self::Token(_) => Some(0.0),
        }
    }
}

/// Font size and other lengths that are not a parent percentage.
#[derive(Clone, Debug, PartialEq)]
pub enum AbsoluteLength {
    Points(f32),
    Rem(f32),
    ViewportWidth(f32),
    ViewportHeight(f32),
    Token(LengthToken),
}

impl AbsoluteLength {
    pub fn points(value: f32) -> Self {
        Self::Points(canonical(value))
    }

    pub fn token(name: &str) -> Self {
        Self::Token(LengthToken::new(name))
    }

    pub fn as_length(&self) -> Length {
        match self {
            Self::Points(value) => Length::points(*value),
            Self::Rem(value) => Length::rem(*value),
            Self::ViewportWidth(value) => Length::ViewportWidth(canonical(*value)),
            Self::ViewportHeight(value) => Length::ViewportHeight(canonical(*value)),
            Self::Token(token) => Length::Token(token.clone()),
        }
    }
}

/// `width` / `height`. `content` is not a member; flex basis has its own type.
#[derive(Clone, Debug, PartialEq)]
pub enum SizeValue {
    Auto,
    Length(Length),
    MinContent,
    MaxContent,
    FitContent,
    FitContentLimit(Length),
    Stretch,
}

impl SizeValue {
    pub fn points(value: f32) -> Self {
        Self::Length(Length::points(value))
    }

    pub fn fraction(value: f32) -> Self {
        Self::Length(Length::fraction(value))
    }

    pub fn auto() -> Self {
        Self::Auto
    }
}

/// `min-*` and `max-*`. `none` on a max axis is [`LimitValue::Auto`].
/// Intrinsic keywords are not representable here.
#[derive(Clone, Debug, PartialEq)]
pub enum LimitValue {
    Auto,
    Length(Length),
}

impl LimitValue {
    pub fn points(value: f32) -> Self {
        Self::Length(Length::points(value))
    }

    pub fn fraction(value: f32) -> Self {
        Self::Length(Length::fraction(value))
    }
}

/// Margin. Negative points are legal. `auto` is not a length.
#[derive(Clone, Debug, PartialEq)]
pub enum MarginValue {
    Auto,
    Length(Length),
}

impl MarginValue {
    pub fn points(value: f32) -> Self {
        Self::Length(Length::points(value))
    }

    pub fn auto() -> Self {
        Self::Auto
    }
}

/// Inset edge. Percentages and negative points are legal.
#[derive(Clone, Debug, PartialEq)]
pub enum InsetValue {
    Auto,
    Length(Length),
}

impl InsetValue {
    pub fn points(value: f32) -> Self {
        Self::Length(Length::points(value))
    }

    pub fn auto() -> Self {
        Self::Auto
    }
}

/// Flex basis. `content` is distinct from [`SizeValue`].
#[derive(Clone, Debug, PartialEq)]
pub enum BasisValue {
    Size(SizeValue),
    Content,
}

impl BasisValue {
    pub fn auto() -> Self {
        Self::Size(SizeValue::Auto)
    }

    pub fn points(value: f32) -> Self {
        Self::Size(SizeValue::points(value))
    }

    pub fn fraction(value: f32) -> Self {
        Self::Size(SizeValue::fraction(value))
    }
}

/// Specified line height. A bare number is a multiplier of the font size.
#[derive(Clone, Debug, PartialEq)]
pub enum LineHeightValue {
    Normal,
    Factor(f32),
    Points(f32),
}

impl LineHeightValue {
    pub fn factor(value: f32) -> Self {
        Self::Factor(canonical(value))
    }
}

/// One bound inside a grid track. `MinMax` cannot nest another `MinMax`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackBound {
    Auto,
    MinContent,
    MaxContent,
    Points(f32),
    /// `fr` count. `1.0` is `1fr`, not 100%.
    Fraction(f32),
    /// Parent percentage, `1.0` = 100%.
    Percent(f32),
}

/// One grid track. `grid-cols-N` expands to `N` copies of `minmax(0, 1fr)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrack {
    Bound(TrackBound),
    MinMax { min: TrackBound, max: TrackBound },
}

impl GridTrack {
    pub const fn fill() -> Self {
        Self::MinMax {
            min: TrackBound::Points(0.0),
            max: TrackBound::Fraction(1.0),
        }
    }
}

/// Typed tracks, or a source string parsed by Taffy's published parser.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackList {
    Tracks(Vec<GridTrack>),
    Source(Arc<str>),
}

impl TrackList {
    pub fn tracks(tracks: Vec<GridTrack>) -> Self {
        Self::Tracks(tracks)
    }

    pub fn source(text: &str) -> Self {
        Self::Source(Arc::from(text))
    }
}

/// Grid line. `Line(0)` and `Span(0)` are rejected at validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridPlacement {
    Auto,
    Line(i16),
    Span(u16),
}

/// `flex` keyword or numeric shorthand before it is expanded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlexShorthand {
    /// `0 1 auto`.
    Initial,
    /// `1 1 auto`.
    Auto,
    /// `0 0 auto`.
    None,
    /// `n 1 0%`. The basis is a percentage, not zero points.
    Number(f32),
}

/// Box display. `Block` is Taffy's block layout, not a flex column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Flex,
    Grid,
    Block,
    FlowRoot,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDir {
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexWrapMode {
    NoWrap,
    Wrap,
    WrapReverse,
}

/// Item alignment. `Start` is logical. `FlexStart` is flex-relative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemAlign {
    Auto,
    Normal,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentAlign {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionMode {
    Relative,
    Absolute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowMode {
    Visible,
    Clip,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxSizingMode {
    BorderBox,
    ContentBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionMode {
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// A color that may still name a theme token. Tokens resolve with the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaintColor {
    Rgba([u8; 4]),
    Token(Arc<str>),
    TokenAlpha(Arc<str>, u8),
}

impl PaintColor {
    pub fn token(name: &str) -> Self {
        Self::Token(Arc::from(name))
    }

    pub fn rgba(color: [u8; 4]) -> Self {
        Self::Rgba(color)
    }
}
