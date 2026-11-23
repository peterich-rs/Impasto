//! Property registry. Names, keys, impact, and inheritance live here once.

use super::values::{
    AbsoluteLength, BasisValue, BoxSizingMode, ContentAlign, DirectionMode, DisplayMode, FlexDir,
    FlexShorthand, FlexWrapMode, GridFlow, GridPlacement, InsetValue, ItemAlign, Length,
    LimitValue, LineHeightValue, MarginValue, OverflowMode, PaintColor, PositionMode, SizeValue,
    TrackBound, TrackList,
};
use super::{Declaration, StyleError};

bitflags::bitflags! {
    /// What a property change invalidates. A color is paint only.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Impact: u8 {
        const LAYOUT = 1 << 0;
        const MEASURE = 1 << 1;
        const PAINT = 1 << 2;
        const LAYER = 1 << 3;
        const HIT = 1 << 4;
        const SEMANTICS = 1 << 5;
    }
}

/// Stable index used inside the style system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum Key {
    Width = 1,
    Height = 2,
    MinWidth = 3,
    MinHeight = 4,
    MaxWidth = 5,
    MaxHeight = 6,
    FlexGrow = 7,
    FlexShrink = 8,
    FlexBasis = 9,
    PaddingTop = 10,
    PaddingRight = 11,
    PaddingBottom = 12,
    PaddingLeft = 13,
    MarginTop = 14,
    MarginRight = 15,
    MarginBottom = 16,
    MarginLeft = 17,
    RowGap = 18,
    ColumnGap = 19,
    Display = 20,
    FlexDirection = 21,
    FlexWrap = 22,
    AlignItems = 23,
    JustifyContent = 24,
    AlignSelf = 25,
    AlignContent = 26,
    JustifyItems = 27,
    JustifySelf = 28,
    Position = 29,
    Top = 30,
    Right = 31,
    Bottom = 32,
    Left = 33,
    AspectRatio = 34,
    GridTemplateColumns = 35,
    GridTemplateRows = 36,
    GridColumnStart = 37,
    GridColumnEnd = 38,
    GridRowStart = 39,
    GridRowEnd = 40,
    BorderWidth = 41,
    BackgroundColor = 50,
    BorderColor = 51,
    OutlineWidth = 52,
    OutlineColor = 53,
    BorderRadius = 54,
    Opacity = 55,
    Overflow = 56,
    TranslateX = 57,
    TranslateY = 58,
    Rotate = 59,
    ScaleX = 60,
    ScaleY = 61,
    Blur = 62,
    Backdrop = 63,
    Blend = 64,
    Shadow = 65,
    BoxShadow = 66,
    Gradient = 67,
    Color = 70,
    FontFamily = 71,
    FontSize = 72,
    FontWeight = 73,
    LineHeight = 74,
    TextAlign = 75,
    WritingDirection = 76,
    MaxLines = 77,
    Ellipsis = 78,
    SoftWrap = 79,
    TextOverflow = 80,
    ObjectFit = 90,
    BoxSizing = 91,
    OverflowX = 92,
    OverflowY = 93,
    ZIndex = 94,
    LayoutDirection = 95,
    TransformOriginX = 96,
    TransformOriginY = 97,
    Underline = 98,
    LineThrough = 99,
    PaddingInlineStart = 100,
    PaddingInlineEnd = 101,
    MarginInlineStart = 102,
    MarginInlineEnd = 103,
    InsetInlineStart = 104,
    InsetInlineEnd = 105,
    GridAutoColumns = 106,
    GridAutoRows = 107,
    GridAutoFlow = 108,
    BorderRadii = 109,
    BorderCurve = 110,
}

impl Key {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Width => "width",
            Self::Height => "height",
            Self::MinWidth => "minWidth",
            Self::MinHeight => "minHeight",
            Self::MaxWidth => "maxWidth",
            Self::MaxHeight => "maxHeight",
            Self::FlexGrow => "flexGrow",
            Self::FlexShrink => "flexShrink",
            Self::FlexBasis => "flexBasis",
            Self::PaddingTop => "paddingTop",
            Self::PaddingRight => "paddingRight",
            Self::PaddingBottom => "paddingBottom",
            Self::PaddingLeft => "paddingLeft",
            Self::MarginTop => "marginTop",
            Self::MarginRight => "marginRight",
            Self::MarginBottom => "marginBottom",
            Self::MarginLeft => "marginLeft",
            Self::RowGap => "rowGap",
            Self::ColumnGap => "columnGap",
            Self::Display => "display",
            Self::FlexDirection => "flexDirection",
            Self::FlexWrap => "flexWrap",
            Self::AlignItems => "alignItems",
            Self::JustifyContent => "justifyContent",
            Self::AlignSelf => "alignSelf",
            Self::AlignContent => "alignContent",
            Self::JustifyItems => "justifyItems",
            Self::JustifySelf => "justifySelf",
            Self::Position => "position",
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::AspectRatio => "aspectRatio",
            Self::GridTemplateColumns => "gridTemplateColumns",
            Self::GridTemplateRows => "gridTemplateRows",
            Self::GridColumnStart => "gridColumnStart",
            Self::GridColumnEnd => "gridColumnEnd",
            Self::GridRowStart => "gridRowStart",
            Self::GridRowEnd => "gridRowEnd",
            Self::BorderWidth => "borderWidth",
            Self::BackgroundColor => "backgroundColor",
            Self::BorderColor => "borderColor",
            Self::OutlineWidth => "outlineWidth",
            Self::OutlineColor => "outlineColor",
            Self::BorderRadius => "borderRadius",
            Self::BorderRadii => "borderRadii",
            Self::BorderCurve => "borderCurve",
            Self::Opacity => "opacity",
            Self::Overflow => "overflow",
            Self::TranslateX => "translateX",
            Self::TranslateY => "translateY",
            Self::Rotate => "rotate",
            Self::ScaleX => "scaleX",
            Self::ScaleY => "scaleY",
            Self::Blur => "blur",
            Self::Backdrop => "backdrop",
            Self::Blend => "blend",
            Self::Shadow => "shadow",
            Self::BoxShadow => "boxShadow",
            Self::Gradient => "gradient",
            Self::Color => "color",
            Self::FontFamily => "fontFamily",
            Self::FontSize => "fontSize",
            Self::FontWeight => "fontWeight",
            Self::LineHeight => "lineHeight",
            Self::TextAlign => "textAlign",
            Self::WritingDirection => "writingDirection",
            Self::MaxLines => "maxLines",
            Self::Ellipsis => "ellipsis",
            Self::SoftWrap => "softWrap",
            Self::TextOverflow => "textOverflow",
            Self::ObjectFit => "objectFit",
            Self::BoxSizing => "boxSizing",
            Self::OverflowX => "overflowX",
            Self::OverflowY => "overflowY",
            Self::ZIndex => "zIndex",
            Self::LayoutDirection => "layoutDirection",
            Self::TransformOriginX => "transformOriginX",
            Self::TransformOriginY => "transformOriginY",
            Self::Underline => "underline",
            Self::LineThrough => "lineThrough",
            Self::PaddingInlineStart => "paddingInlineStart",
            Self::PaddingInlineEnd => "paddingInlineEnd",
            Self::MarginInlineStart => "marginInlineStart",
            Self::MarginInlineEnd => "marginInlineEnd",
            Self::InsetInlineStart => "insetInlineStart",
            Self::InsetInlineEnd => "insetInlineEnd",
            Self::GridAutoColumns => "gridAutoColumns",
            Self::GridAutoRows => "gridAutoRows",
            Self::GridAutoFlow => "gridAutoFlow",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        KEYS.iter().copied().find(|key| key.name() == name)
    }

    pub const fn code(self) -> u16 {
        self as u16
    }

    pub fn impact(self) -> Impact {
        match self {
            Self::Color
            | Self::BackgroundColor
            | Self::BorderColor
            | Self::OutlineColor
            | Self::Shadow
            | Self::BoxShadow
            | Self::Gradient
            | Self::Underline
            | Self::LineThrough => Impact::PAINT,
            Self::Opacity
            | Self::TranslateX
            | Self::TranslateY
            | Self::Rotate
            | Self::ScaleX
            | Self::ScaleY
            | Self::TransformOriginX
            | Self::TransformOriginY
            | Self::Blur
            | Self::Backdrop
            | Self::Blend => Impact::LAYER | Impact::PAINT | Impact::HIT,
            Self::BorderRadius | Self::BorderRadii | Self::BorderCurve | Self::OutlineWidth => {
                Impact::PAINT | Impact::HIT
            }
            Self::ZIndex => Impact::PAINT | Impact::HIT,
            Self::Overflow | Self::OverflowX | Self::OverflowY => {
                Impact::LAYOUT | Impact::HIT | Impact::PAINT | Impact::SEMANTICS
            }
            Self::FontSize
            | Self::FontWeight
            | Self::FontFamily
            | Self::LineHeight
            | Self::WritingDirection
            | Self::MaxLines
            | Self::Ellipsis
            | Self::SoftWrap
            | Self::TextOverflow
            | Self::TextAlign => Impact::MEASURE | Impact::PAINT,
            Self::ObjectFit => Impact::PAINT,
            Self::Display => Impact::LAYOUT | Impact::HIT | Impact::PAINT | Impact::SEMANTICS,
            Self::LayoutDirection => Impact::LAYOUT | Impact::HIT,
            _ => Impact::LAYOUT,
        }
    }

    /// A child takes this from an ancestor when it has no own value and no reset.
    pub const fn inherited(self) -> bool {
        matches!(
            self,
            Self::Color
                | Self::FontFamily
                | Self::FontSize
                | Self::FontWeight
                | Self::LineHeight
                | Self::TextAlign
                | Self::WritingDirection
                | Self::LayoutDirection
        )
    }
}

pub(super) const KEYS: &[Key] = &[
    Key::Width,
    Key::Height,
    Key::MinWidth,
    Key::MinHeight,
    Key::MaxWidth,
    Key::MaxHeight,
    Key::FlexGrow,
    Key::FlexShrink,
    Key::FlexBasis,
    Key::PaddingTop,
    Key::PaddingRight,
    Key::PaddingBottom,
    Key::PaddingLeft,
    Key::MarginTop,
    Key::MarginRight,
    Key::MarginBottom,
    Key::MarginLeft,
    Key::RowGap,
    Key::ColumnGap,
    Key::Display,
    Key::FlexDirection,
    Key::FlexWrap,
    Key::AlignItems,
    Key::JustifyContent,
    Key::AlignSelf,
    Key::AlignContent,
    Key::JustifyItems,
    Key::JustifySelf,
    Key::Position,
    Key::Top,
    Key::Right,
    Key::Bottom,
    Key::Left,
    Key::AspectRatio,
    Key::GridTemplateColumns,
    Key::GridTemplateRows,
    Key::GridColumnStart,
    Key::GridColumnEnd,
    Key::GridRowStart,
    Key::GridRowEnd,
    Key::BorderWidth,
    Key::BackgroundColor,
    Key::BorderColor,
    Key::OutlineWidth,
    Key::OutlineColor,
    Key::BorderRadius,
    Key::BorderRadii,
    Key::BorderCurve,
    Key::Opacity,
    Key::Overflow,
    Key::TranslateX,
    Key::TranslateY,
    Key::Rotate,
    Key::ScaleX,
    Key::ScaleY,
    Key::Blur,
    Key::Backdrop,
    Key::Blend,
    Key::Shadow,
    Key::BoxShadow,
    Key::Gradient,
    Key::Color,
    Key::FontFamily,
    Key::FontSize,
    Key::FontWeight,
    Key::LineHeight,
    Key::TextAlign,
    Key::WritingDirection,
    Key::MaxLines,
    Key::Ellipsis,
    Key::SoftWrap,
    Key::TextOverflow,
    Key::ObjectFit,
    Key::BoxSizing,
    Key::OverflowX,
    Key::OverflowY,
    Key::ZIndex,
    Key::LayoutDirection,
    Key::TransformOriginX,
    Key::TransformOriginY,
    Key::Underline,
    Key::LineThrough,
    Key::PaddingInlineStart,
    Key::PaddingInlineEnd,
    Key::MarginInlineStart,
    Key::MarginInlineEnd,
    Key::InsetInlineStart,
    Key::InsetInlineEnd,
    Key::GridAutoColumns,
    Key::GridAutoRows,
    Key::GridAutoFlow,
];

/// One longhand value. Shorthands expand before a declaration is stored.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Property {
    Width(SizeValue),
    Height(SizeValue),
    MinWidth(LimitValue),
    MinHeight(LimitValue),
    MaxWidth(LimitValue),
    MaxHeight(LimitValue),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(BasisValue),
    PaddingTop(Length),
    PaddingRight(Length),
    PaddingBottom(Length),
    PaddingLeft(Length),
    MarginTop(MarginValue),
    MarginRight(MarginValue),
    MarginBottom(MarginValue),
    MarginLeft(MarginValue),
    RowGap(Length),
    ColumnGap(Length),
    Display(DisplayMode),
    FlexDirection(FlexDir),
    FlexWrap(FlexWrapMode),
    AlignItems(ItemAlign),
    JustifyContent(ContentAlign),
    AlignSelf(ItemAlign),
    AlignContent(ContentAlign),
    JustifyItems(ItemAlign),
    JustifySelf(ItemAlign),
    Position(PositionMode),
    Top(InsetValue),
    Right(InsetValue),
    Bottom(InsetValue),
    Left(InsetValue),
    AspectRatio(f32),
    GridTemplateColumns(TrackList),
    GridTemplateRows(TrackList),
    GridColumnStart(GridPlacement),
    GridColumnEnd(GridPlacement),
    GridRowStart(GridPlacement),
    GridRowEnd(GridPlacement),
    BorderWidth(Length),
    BackgroundColor(PaintColor),
    BorderColor(PaintColor),
    OutlineWidth(Length),
    OutlineColor(PaintColor),
    BorderRadius(Length),
    BorderRadii([f32; 4]),
    BorderCurve(f32),
    /// Author opacity in `0..=1`. `1` stays on the fast path.
    Opacity(f32),
    Overflow(OverflowMode),
    TranslateX(Length),
    TranslateY(Length),
    /// Radians.
    Rotate(f32),
    ScaleX(f32),
    ScaleY(f32),
    Blur(f32),
    Backdrop(f32),
    Blend(crate::scene::BlendMode),
    Shadow(crate::scene::Shadow),
    BoxShadow(Vec<crate::scene::BoxShadow>),
    Gradient(crate::scene::Gradient),
    Color(PaintColor),
    FontFamily(String),
    FontSize(AbsoluteLength),
    FontWeight(u16),
    LineHeight(LineHeightValue),
    TextAlign(crate::scene::TextAlign),
    WritingDirectionRtl(bool),
    MaxLines(u32),
    Ellipsis(String),
    SoftWrap(bool),
    TextOverflow(crate::scene::TextOverflow),
    ObjectFit(crate::scene::ContentMode),
    BoxSizing(BoxSizingMode),
    OverflowX(OverflowMode),
    OverflowY(OverflowMode),
    ZIndex(i32),
    LayoutDirection(DirectionMode),
    TransformOriginX(Length),
    TransformOriginY(Length),
    Underline(bool),
    LineThrough(bool),
    PaddingInlineStart(Length),
    PaddingInlineEnd(Length),
    MarginInlineStart(MarginValue),
    MarginInlineEnd(MarginValue),
    InsetInlineStart(InsetValue),
    InsetInlineEnd(InsetValue),
    GridAutoColumns(TrackList),
    GridAutoRows(TrackList),
    GridAutoFlow(GridFlow),
}

impl Property {
    pub fn key(&self) -> Key {
        match self {
            Self::Width(_) => Key::Width,
            Self::Height(_) => Key::Height,
            Self::MinWidth(_) => Key::MinWidth,
            Self::MinHeight(_) => Key::MinHeight,
            Self::MaxWidth(_) => Key::MaxWidth,
            Self::MaxHeight(_) => Key::MaxHeight,
            Self::FlexGrow(_) => Key::FlexGrow,
            Self::FlexShrink(_) => Key::FlexShrink,
            Self::FlexBasis(_) => Key::FlexBasis,
            Self::PaddingTop(_) => Key::PaddingTop,
            Self::PaddingRight(_) => Key::PaddingRight,
            Self::PaddingBottom(_) => Key::PaddingBottom,
            Self::PaddingLeft(_) => Key::PaddingLeft,
            Self::MarginTop(_) => Key::MarginTop,
            Self::MarginRight(_) => Key::MarginRight,
            Self::MarginBottom(_) => Key::MarginBottom,
            Self::MarginLeft(_) => Key::MarginLeft,
            Self::RowGap(_) => Key::RowGap,
            Self::ColumnGap(_) => Key::ColumnGap,
            Self::Display(_) => Key::Display,
            Self::FlexDirection(_) => Key::FlexDirection,
            Self::FlexWrap(_) => Key::FlexWrap,
            Self::AlignItems(_) => Key::AlignItems,
            Self::JustifyContent(_) => Key::JustifyContent,
            Self::AlignSelf(_) => Key::AlignSelf,
            Self::AlignContent(_) => Key::AlignContent,
            Self::JustifyItems(_) => Key::JustifyItems,
            Self::JustifySelf(_) => Key::JustifySelf,
            Self::Position(_) => Key::Position,
            Self::Top(_) => Key::Top,
            Self::Right(_) => Key::Right,
            Self::Bottom(_) => Key::Bottom,
            Self::Left(_) => Key::Left,
            Self::AspectRatio(_) => Key::AspectRatio,
            Self::GridTemplateColumns(_) => Key::GridTemplateColumns,
            Self::GridTemplateRows(_) => Key::GridTemplateRows,
            Self::GridColumnStart(_) => Key::GridColumnStart,
            Self::GridColumnEnd(_) => Key::GridColumnEnd,
            Self::GridRowStart(_) => Key::GridRowStart,
            Self::GridRowEnd(_) => Key::GridRowEnd,
            Self::BorderWidth(_) => Key::BorderWidth,
            Self::BackgroundColor(_) => Key::BackgroundColor,
            Self::BorderColor(_) => Key::BorderColor,
            Self::OutlineWidth(_) => Key::OutlineWidth,
            Self::OutlineColor(_) => Key::OutlineColor,
            Self::BorderRadius(_) => Key::BorderRadius,
            Self::BorderRadii(_) => Key::BorderRadii,
            Self::BorderCurve(_) => Key::BorderCurve,
            Self::Opacity(_) => Key::Opacity,
            Self::Overflow(_) => Key::Overflow,
            Self::TranslateX(_) => Key::TranslateX,
            Self::TranslateY(_) => Key::TranslateY,
            Self::Rotate(_) => Key::Rotate,
            Self::ScaleX(_) => Key::ScaleX,
            Self::ScaleY(_) => Key::ScaleY,
            Self::Blur(_) => Key::Blur,
            Self::Backdrop(_) => Key::Backdrop,
            Self::Blend(_) => Key::Blend,
            Self::Shadow(_) => Key::Shadow,
            Self::BoxShadow(_) => Key::BoxShadow,
            Self::Gradient(_) => Key::Gradient,
            Self::Color(_) => Key::Color,
            Self::FontFamily(_) => Key::FontFamily,
            Self::FontSize(_) => Key::FontSize,
            Self::FontWeight(_) => Key::FontWeight,
            Self::LineHeight(_) => Key::LineHeight,
            Self::TextAlign(_) => Key::TextAlign,
            Self::WritingDirectionRtl(_) => Key::WritingDirection,
            Self::MaxLines(_) => Key::MaxLines,
            Self::Ellipsis(_) => Key::Ellipsis,
            Self::SoftWrap(_) => Key::SoftWrap,
            Self::TextOverflow(_) => Key::TextOverflow,
            Self::ObjectFit(_) => Key::ObjectFit,
            Self::BoxSizing(_) => Key::BoxSizing,
            Self::OverflowX(_) => Key::OverflowX,
            Self::OverflowY(_) => Key::OverflowY,
            Self::ZIndex(_) => Key::ZIndex,
            Self::LayoutDirection(_) => Key::LayoutDirection,
            Self::TransformOriginX(_) => Key::TransformOriginX,
            Self::TransformOriginY(_) => Key::TransformOriginY,
            Self::Underline(_) => Key::Underline,
            Self::LineThrough(_) => Key::LineThrough,
            Self::PaddingInlineStart(_) => Key::PaddingInlineStart,
            Self::PaddingInlineEnd(_) => Key::PaddingInlineEnd,
            Self::MarginInlineStart(_) => Key::MarginInlineStart,
            Self::MarginInlineEnd(_) => Key::MarginInlineEnd,
            Self::InsetInlineStart(_) => Key::InsetInlineStart,
            Self::InsetInlineEnd(_) => Key::InsetInlineEnd,
            Self::GridAutoColumns(_) => Key::GridAutoColumns,
            Self::GridAutoRows(_) => Key::GridAutoRows,
            Self::GridAutoFlow(_) => Key::GridAutoFlow,
        }
    }

    pub fn validate(&self) -> Result<(), StyleError> {
        match self {
            Self::Width(value) | Self::Height(value) => size_ok(value, self.key()),
            Self::MinWidth(value)
            | Self::MinHeight(value)
            | Self::MaxWidth(value)
            | Self::MaxHeight(value) => limit_ok(value, self.key()),
            Self::FlexBasis(value) => basis_ok(value),
            Self::FlexGrow(value) | Self::FlexShrink(value) => non_negative(*value, self.key()),
            Self::PaddingTop(value)
            | Self::PaddingRight(value)
            | Self::PaddingBottom(value)
            | Self::PaddingLeft(value)
            | Self::PaddingInlineStart(value)
            | Self::PaddingInlineEnd(value)
            | Self::RowGap(value)
            | Self::ColumnGap(value)
            | Self::BorderWidth(value)
            | Self::OutlineWidth(value)
            | Self::BorderRadius(value) => length_non_negative(value, self.key()),
            Self::BorderRadii(values) => values
                .iter()
                .try_for_each(|value| non_negative(*value, self.key())),
            Self::BorderCurve(value) if (2.0..=8.0).contains(value) => Ok(()),
            Self::BorderCurve(_) => bad(self.key(), "expected a value between 2 and 8"),
            Self::MarginTop(value)
            | Self::MarginRight(value)
            | Self::MarginBottom(value)
            | Self::MarginLeft(value)
            | Self::MarginInlineStart(value)
            | Self::MarginInlineEnd(value) => margin_ok(value, self.key()),
            Self::Top(value)
            | Self::Right(value)
            | Self::Bottom(value)
            | Self::Left(value)
            | Self::InsetInlineStart(value)
            | Self::InsetInlineEnd(value) => inset_ok(value, self.key()),
            Self::TransformOriginX(value) | Self::TransformOriginY(value) => {
                length_finite(value, self.key())
            }
            Self::AspectRatio(value) => positive(*value, self.key()),
            Self::FontSize(value) => absolute_positive(value),
            Self::LineHeight(value) => line_height_ok(value),
            Self::Opacity(value) => unit_interval(*value),
            Self::FontWeight(value) => font_weight(*value),
            Self::TranslateX(value) | Self::TranslateY(value) => length_finite(value, self.key()),
            Self::Rotate(value) | Self::ScaleX(value) | Self::ScaleY(value) => {
                finite(*value, self.key())
            }
            Self::Blur(value) | Self::Backdrop(value) => non_negative(*value, self.key()),
            Self::GridColumnStart(line)
            | Self::GridColumnEnd(line)
            | Self::GridRowStart(line)
            | Self::GridRowEnd(line) => grid_line(*line, self.key()),
            Self::GridTemplateColumns(tracks)
            | Self::GridTemplateRows(tracks)
            | Self::GridAutoColumns(tracks)
            | Self::GridAutoRows(tracks) => tracks_ok(tracks, self.key()),
            Self::Shadow(shadow) => non_negative(shadow.elevation, self.key()),
            Self::BoxShadow(shadows) => {
                for shadow in shadows {
                    for value in shadow.offset.into_iter().chain([shadow.spread]) {
                        finite(value, self.key())?;
                    }
                    non_negative(shadow.blur_sigma, self.key())?;
                }
                Ok(())
            }
            Self::Gradient(gradient) => {
                let stops = match gradient {
                    crate::scene::Gradient::Linear { start, end, stops } => {
                        for value in start.iter().chain(end) {
                            finite(*value, self.key())?;
                        }
                        stops
                    }
                    crate::scene::Gradient::Radial {
                        center,
                        radius,
                        stops,
                    } => {
                        for value in center {
                            finite(*value, self.key())?;
                        }
                        non_negative(*radius, self.key())?;
                        stops
                    }
                    crate::scene::Gradient::Sweep {
                        center,
                        start_angle_rad,
                        end_angle_rad,
                        stops,
                    } => {
                        for value in center.iter().chain([start_angle_rad, end_angle_rad]) {
                            finite(*value, self.key())?;
                        }
                        stops
                    }
                };
                for stop in stops {
                    unit_interval(stop.offset)?;
                }
                Ok(())
            }
            Self::AlignItems(ItemAlign::Auto) | Self::JustifyItems(ItemAlign::Auto) => {
                bad(self.key(), "auto is only valid for self alignment")
            }
            _ => Ok(()),
        }
    }
}

fn size_ok(value: &SizeValue, key: Key) -> Result<(), StyleError> {
    match value {
        SizeValue::Auto
        | SizeValue::MinContent
        | SizeValue::MaxContent
        | SizeValue::FitContent
        | SizeValue::Stretch => Ok(()),
        SizeValue::Length(length) | SizeValue::FitContentLimit(length) => {
            length_non_negative(length, key)
        }
    }
}

fn limit_ok(value: &LimitValue, key: Key) -> Result<(), StyleError> {
    match value {
        LimitValue::Auto => Ok(()),
        LimitValue::Length(length) => length_non_negative(length, key),
    }
}

fn basis_ok(value: &BasisValue) -> Result<(), StyleError> {
    match value {
        BasisValue::Content => Ok(()),
        BasisValue::Size(size) => size_ok(size, Key::FlexBasis),
    }
}

fn margin_ok(value: &MarginValue, key: Key) -> Result<(), StyleError> {
    match value {
        MarginValue::Auto => Ok(()),
        MarginValue::Length(length) => length_finite(length, key),
    }
}

fn inset_ok(value: &InsetValue, key: Key) -> Result<(), StyleError> {
    match value {
        InsetValue::Auto => Ok(()),
        InsetValue::Length(length) => length_finite(length, key),
    }
}

fn length_finite(value: &Length, key: Key) -> Result<(), StyleError> {
    match value {
        Length::Token(_) => Ok(()),
        Length::Points(number)
        | Length::Fraction(number)
        | Length::Rem(number)
        | Length::ViewportWidth(number)
        | Length::ViewportHeight(number)
        | Length::Steps(number) => finite(*number, key),
    }
}

fn length_non_negative(value: &Length, key: Key) -> Result<(), StyleError> {
    match value {
        Length::Token(_) => Ok(()),
        Length::Points(number)
        | Length::Fraction(number)
        | Length::Rem(number)
        | Length::ViewportWidth(number)
        | Length::ViewportHeight(number)
        | Length::Steps(number) => non_negative(*number, key),
    }
}

fn absolute_positive(value: &AbsoluteLength) -> Result<(), StyleError> {
    match value {
        AbsoluteLength::Token(_) => Ok(()),
        AbsoluteLength::Points(number)
        | AbsoluteLength::Rem(number)
        | AbsoluteLength::ViewportWidth(number)
        | AbsoluteLength::ViewportHeight(number) => positive(*number, Key::FontSize),
    }
}

fn line_height_ok(value: &LineHeightValue) -> Result<(), StyleError> {
    match value {
        LineHeightValue::Normal => Ok(()),
        LineHeightValue::Factor(number) | LineHeightValue::Points(number) => {
            non_negative(*number, Key::LineHeight)
        }
    }
}

fn finite(value: f32, key: Key) -> Result<(), StyleError> {
    if value.is_finite() {
        Ok(())
    } else {
        bad(key, "number is not finite")
    }
}

fn non_negative(value: f32, key: Key) -> Result<(), StyleError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        bad(key, "number must be finite and >= 0")
    }
}

fn positive(value: f32, key: Key) -> Result<(), StyleError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        bad(key, "number must be finite and > 0")
    }
}

fn unit_interval(value: f32) -> Result<(), StyleError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        bad(Key::Opacity, "opacity must be in 0..=1")
    }
}

fn font_weight(value: u16) -> Result<(), StyleError> {
    if (100..=900).contains(&value) && value.is_multiple_of(100) {
        Ok(())
    } else {
        bad(
            Key::FontWeight,
            "weight must be a multiple of 100 from 100 to 900",
        )
    }
}

fn grid_line(line: GridPlacement, key: Key) -> Result<(), StyleError> {
    match line {
        GridPlacement::Auto => Ok(()),
        GridPlacement::Line(index) if index != 0 => Ok(()),
        GridPlacement::Span(count) if count != 0 => Ok(()),
        _ => bad(key, "grid line cannot be 0"),
    }
}

fn tracks_ok(tracks: &TrackList, key: Key) -> Result<(), StyleError> {
    match tracks {
        TrackList::Tracks(tracks) => {
            if tracks.is_empty() {
                return bad(key, "track list is empty");
            }
            for track in tracks {
                track_ok(*track, key)?;
            }
            Ok(())
        }
        TrackList::Source(source) => {
            super::taffy::accept_track_source(source).map_err(|_| StyleError::Invalid {
                node: None,
                property: key.name(),
                reason: "grid track syntax is not supported",
            })
        }
    }
}

fn track_ok(track: super::values::GridTrack, key: Key) -> Result<(), StyleError> {
    match track {
        super::values::GridTrack::Bound(bound) => bound_ok(bound, key),
        super::values::GridTrack::MinMax { min, max } => {
            if matches!(min, TrackBound::Fraction(_)) {
                return bad(key, "minmax min cannot be fr");
            }
            bound_ok(min, key)?;
            bound_ok(max, key)
        }
    }
}

fn bound_ok(bound: TrackBound, key: Key) -> Result<(), StyleError> {
    match bound {
        TrackBound::Auto | TrackBound::MinContent | TrackBound::MaxContent => Ok(()),
        TrackBound::Points(value) | TrackBound::Fraction(value) | TrackBound::Percent(value) => {
            non_negative(value, key)
        }
    }
}

pub(crate) fn bad(key: Key, reason: &'static str) -> Result<(), StyleError> {
    Err(StyleError::Invalid {
        node: None,
        property: key.name(),
        reason,
    })
}

pub fn expand_flex(flex: FlexShorthand) -> Result<Vec<Declaration>, StyleError> {
    let (grow, shrink, basis) = match flex {
        FlexShorthand::None => (0.0, 0.0, BasisValue::auto()),
        FlexShorthand::Auto => (1.0, 1.0, BasisValue::auto()),
        FlexShorthand::Initial => (0.0, 1.0, BasisValue::auto()),
        FlexShorthand::Number(value) => {
            non_negative(value, Key::FlexGrow)?;
            (value, 1.0, BasisValue::fraction(0.0))
        }
    };
    Ok(vec![
        Declaration::set(Property::FlexGrow(grow)),
        Declaration::set(Property::FlexShrink(shrink)),
        Declaration::set(Property::FlexBasis(basis)),
    ])
}

pub fn expand_padding(value: Length) -> Result<Vec<Declaration>, StyleError> {
    length_non_negative(&value, Key::PaddingTop)?;
    Ok(edges(
        Property::PaddingTop(value.clone()),
        Property::PaddingRight(value.clone()),
        Property::PaddingBottom(value.clone()),
        Property::PaddingLeft(value),
    ))
}

pub fn expand_margin(value: MarginValue) -> Result<Vec<Declaration>, StyleError> {
    margin_ok(&value, Key::MarginTop)?;
    Ok(edges(
        Property::MarginTop(value.clone()),
        Property::MarginRight(value.clone()),
        Property::MarginBottom(value.clone()),
        Property::MarginLeft(value),
    ))
}

pub fn expand_gap(value: Length) -> Result<Vec<Declaration>, StyleError> {
    length_non_negative(&value, Key::RowGap)?;
    Ok(vec![
        Declaration::set(Property::RowGap(value.clone())),
        Declaration::set(Property::ColumnGap(value)),
    ])
}

pub fn expand_inset(value: InsetValue) -> Result<Vec<Declaration>, StyleError> {
    inset_ok(&value, Key::Top)?;
    Ok(edges(
        Property::Top(value.clone()),
        Property::Right(value.clone()),
        Property::Bottom(value.clone()),
        Property::Left(value),
    ))
}

fn edges(top: Property, right: Property, bottom: Property, left: Property) -> Vec<Declaration> {
    vec![
        Declaration::set(top),
        Declaration::set(right),
        Declaration::set(bottom),
        Declaration::set(left),
    ]
}

/// `display: flex` sets only the display. Direction comes from the node preset
/// or from an explicit `flex-row` / `flex-col`.
pub fn display_flex() -> Vec<Declaration> {
    vec![Declaration::set(Property::Display(DisplayMode::Flex))]
}

/// Metadata the type generator prints. Order is the author-facing order.
pub struct FieldMeta {
    pub name: &'static str,
    pub ts_type: &'static str,
    pub group: &'static str,
}

pub fn field_metas() -> &'static [FieldMeta] {
    FIELDS
}

const FIELDS: &[FieldMeta] = &[
    FieldMeta {
        name: "width",
        ts_type: "SizeValue",
        group: "box",
    },
    FieldMeta {
        name: "height",
        ts_type: "SizeValue",
        group: "box",
    },
    FieldMeta {
        name: "minWidth",
        ts_type: "MinSize",
        group: "box",
    },
    FieldMeta {
        name: "minHeight",
        ts_type: "MinSize",
        group: "box",
    },
    FieldMeta {
        name: "maxWidth",
        ts_type: "MaxSize",
        group: "box",
    },
    FieldMeta {
        name: "maxHeight",
        ts_type: "MaxSize",
        group: "box",
    },
    FieldMeta {
        name: "flex",
        ts_type: "FlexValue",
        group: "box",
    },
    FieldMeta {
        name: "flexGrow",
        ts_type: "number",
        group: "box",
    },
    FieldMeta {
        name: "flexShrink",
        ts_type: "number",
        group: "box",
    },
    FieldMeta {
        name: "flexBasis",
        ts_type: "BasisValue",
        group: "box",
    },
    FieldMeta {
        name: "padding",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingTop",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingRight",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingBottom",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingLeft",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingInlineStart",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "paddingInlineEnd",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "margin",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginTop",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginRight",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginBottom",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginLeft",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginInlineStart",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "marginInlineEnd",
        ts_type: "MarginValue",
        group: "box",
    },
    FieldMeta {
        name: "gap",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "rowGap",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "columnGap",
        ts_type: "Length",
        group: "box",
    },
    FieldMeta {
        name: "display",
        ts_type: "\"flex\" | \"grid\" | \"block\" | \"flow-root\" | \"none\"",
        group: "box",
    },
    FieldMeta {
        name: "flexDirection",
        ts_type: "\"row\" | \"column\" | \"row-reverse\" | \"column-reverse\"",
        group: "box",
    },
    FieldMeta {
        name: "flexWrap",
        ts_type: "\"nowrap\" | \"wrap\" | \"wrap-reverse\"",
        group: "box",
    },
    FieldMeta {
        name: "alignItems",
        ts_type: "\"normal\" | \"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"baseline\"",
        group: "box",
    },
    FieldMeta {
        name: "justifyContent",
        ts_type: "\"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"space-between\" | \"space-around\" | \"space-evenly\"",
        group: "box",
    },
    FieldMeta {
        name: "alignSelf",
        ts_type: "\"auto\" | \"normal\" | \"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"baseline\"",
        group: "box",
    },
    FieldMeta {
        name: "alignContent",
        ts_type: "\"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"space-between\" | \"space-around\" | \"space-evenly\"",
        group: "box",
    },
    FieldMeta {
        name: "justifyItems",
        ts_type: "\"normal\" | \"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"baseline\"",
        group: "box",
    },
    FieldMeta {
        name: "justifySelf",
        ts_type: "\"auto\" | \"normal\" | \"start\" | \"end\" | \"flex-start\" | \"flex-end\" | \"center\" | \"stretch\" | \"baseline\"",
        group: "box",
    },
    FieldMeta {
        name: "position",
        ts_type: "\"relative\" | \"absolute\"",
        group: "box",
    },
    FieldMeta {
        name: "inset",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "top",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "right",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "bottom",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "left",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "insetInlineStart",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "insetInlineEnd",
        ts_type: "InsetValue",
        group: "box",
    },
    FieldMeta {
        name: "aspectRatio",
        ts_type: "number",
        group: "box",
    },
    FieldMeta {
        name: "boxSizing",
        ts_type: "\"border-box\" | \"content-box\"",
        group: "box",
    },
    FieldMeta {
        name: "gridTemplateColumns",
        ts_type: "readonly GridTrack[] | string",
        group: "box",
    },
    FieldMeta {
        name: "gridTemplateRows",
        ts_type: "readonly GridTrack[] | string",
        group: "box",
    },
    FieldMeta {
        name: "gridAutoColumns",
        ts_type: "readonly GridTrack[] | string",
        group: "box",
    },
    FieldMeta {
        name: "gridAutoRows",
        ts_type: "readonly GridTrack[] | string",
        group: "box",
    },
    FieldMeta {
        name: "gridAutoFlow",
        ts_type: "\"row\" | \"column\" | \"row dense\" | \"column dense\"",
        group: "box",
    },
    FieldMeta {
        name: "gridColumnStart",
        ts_type: "GridLine",
        group: "box",
    },
    FieldMeta {
        name: "gridColumnEnd",
        ts_type: "GridLine",
        group: "box",
    },
    FieldMeta {
        name: "gridRowStart",
        ts_type: "GridLine",
        group: "box",
    },
    FieldMeta {
        name: "gridRowEnd",
        ts_type: "GridLine",
        group: "box",
    },
    FieldMeta {
        name: "overflow",
        ts_type: "\"visible\" | \"clip\" | \"hidden\"",
        group: "box",
    },
    FieldMeta {
        name: "overflowX",
        ts_type: "\"visible\" | \"clip\" | \"hidden\"",
        group: "box",
    },
    FieldMeta {
        name: "overflowY",
        ts_type: "\"visible\" | \"clip\" | \"hidden\"",
        group: "box",
    },
    FieldMeta {
        name: "layoutDirection",
        ts_type: "Inherited<\"ltr\" | \"rtl\">",
        group: "box",
    },
    FieldMeta {
        name: "zIndex",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "backgroundColor",
        ts_type: "Color",
        group: "paint",
    },
    FieldMeta {
        name: "borderColor",
        ts_type: "Color",
        group: "paint",
    },
    FieldMeta {
        name: "borderWidth",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "border",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "outlineWidth",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "outlineColor",
        ts_type: "Color",
        group: "paint",
    },
    FieldMeta {
        name: "borderRadius",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "borderRadii",
        ts_type: "readonly [number, number, number, number]",
        group: "paint",
    },
    FieldMeta {
        name: "borderCurve",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "opacity",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "translateX",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "translateY",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "rotate",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "scale",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "scaleX",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "scaleY",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "transformOriginX",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "transformOriginY",
        ts_type: "Length",
        group: "paint",
    },
    FieldMeta {
        name: "shadow",
        ts_type: "ShadowStyle",
        group: "paint",
    },
    FieldMeta {
        name: "boxShadow",
        ts_type: "readonly BoxShadowStyle[]",
        group: "paint",
    },
    FieldMeta {
        name: "gradient",
        ts_type: "GradientStyle",
        group: "paint",
    },
    FieldMeta {
        name: "blur",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "backdrop",
        ts_type: "number",
        group: "paint",
    },
    FieldMeta {
        name: "blend",
        ts_type: "\"src-over\" | \"multiply\" | \"screen\"",
        group: "paint",
    },
    FieldMeta {
        name: "color",
        ts_type: "Inherited<Color>",
        group: "text",
    },
    FieldMeta {
        name: "fontSize",
        ts_type: "Inherited<AbsoluteLength>",
        group: "text",
    },
    FieldMeta {
        name: "fontWeight",
        ts_type: "Inherited<number>",
        group: "text",
    },
    FieldMeta {
        name: "fontFamily",
        ts_type: "Inherited<string>",
        group: "text",
    },
    FieldMeta {
        name: "textAlign",
        ts_type: "Inherited<\"left\" | \"center\" | \"right\">",
        group: "text",
    },
    FieldMeta {
        name: "writingDirection",
        ts_type: "Inherited<\"ltr\" | \"rtl\">",
        group: "text",
    },
    FieldMeta {
        name: "lineHeight",
        ts_type: "Inherited<number | \"normal\" | { points: number }>",
        group: "text",
    },
    FieldMeta {
        name: "maxLines",
        ts_type: "number",
        group: "text",
    },
    FieldMeta {
        name: "ellipsis",
        ts_type: "string",
        group: "text",
    },
    FieldMeta {
        name: "softWrap",
        ts_type: "boolean",
        group: "text",
    },
    FieldMeta {
        name: "textOverflow",
        ts_type: "\"clip\" | \"ellipsis\" | \"visible\"",
        group: "text",
    },
    FieldMeta {
        name: "underline",
        ts_type: "boolean",
        group: "text",
    },
    FieldMeta {
        name: "lineThrough",
        ts_type: "boolean",
        group: "text",
    },
    FieldMeta {
        name: "objectFit",
        ts_type: "\"fill\" | \"contain\" | \"cover\"",
        group: "paint",
    },
];
