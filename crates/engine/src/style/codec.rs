//! Typed snapshots copied from the script boundary. Lengths remain unresolved.

use super::property::{expand_flex, expand_gap, expand_inset, expand_margin, expand_padding};
use super::values::{
    AbsoluteLength, BasisValue, ContentAlign, DirectionMode, DisplayMode, FlexDir, FlexShorthand,
    FlexWrapMode, InsetValue, ItemAlign, Length, LimitValue, LineHeightValue, MarginValue,
    OverflowMode, PaintColor, PositionMode, SizeValue, TrackList,
};
use super::{Declaration, Key, Property, StyleError};

/// Owned values copied from the generated native records before a tree borrow.
#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Number(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}
impl Value {
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }
    fn number(&self, name: &str) -> Result<f32, StyleError> {
        match self {
            Self::Number(value) => finite_f32(name, *value),
            _ => Err(invalid(name, "expected a number")),
        }
    }
}

/// Snapshot fragments are validated in full before one source is replaced.
pub fn from_value(value: &Value) -> Result<super::Style, StyleError> {
    fn fragment(value: &Value, style: &mut super::Style) -> Result<(), StyleError> {
        match value {
            Value::Null | Value::Undefined | Value::Bool(false) => Ok(()),
            Value::Array(items) => {
                for item in items {
                    fragment(item, style)?;
                }
                Ok(())
            }
            Value::Object(fields) => {
                let mut short = Vec::new();
                let mut long = Vec::new();
                for (name, value) in fields {
                    if matches!(value, Value::Undefined) {
                        continue;
                    }
                    let declarations = declarations(name, value)?;
                    if is_shorthand(name) {
                        short.extend(declarations);
                    } else {
                        long.extend(declarations);
                    }
                }
                short.extend(long);
                style.push_fragment(short)
            }
            _ => Err(invalid("style", "expected an object or fragment array")),
        }
    }
    let mut style = super::Style::new();
    fragment(value, &mut style)?;
    Ok(style)
}

pub fn declarations(name: &str, value: &Value) -> Result<Vec<Declaration>, StyleError> {
    use super::values::{GridPlacement, GridTrack};
    match value {
        Value::Null => resets(name),
        Value::Undefined => Ok(Vec::new()),
        Value::Array(values) if name == "borderRadii" => {
            let [tl, tr, br, bl] = values.as_slice() else {
                return Err(invalid(name, "expected four corner radii"));
            };
            set(Property::BorderRadii([
                tl.number(name)?,
                tr.number(name)?,
                br.number(name)?,
                bl.number(name)?,
            ]))
        }
        Value::Number(value) if name == "borderCurve" => {
            set(Property::BorderCurve(finite_f32(name, *value)?))
        }
        Value::Number(value) => number_value(name, finite_f32(name, *value)?),
        Value::Bool(value) => match name {
            "softWrap" => set(Property::SoftWrap(*value)),
            "underline" => set(Property::Underline(*value)),
            "lineThrough" => set(Property::LineThrough(*value)),
            _ => Err(invalid(name, "expected a non-boolean style value")),
        },
        Value::String(word) => string_value(name, word),
        Value::Array(items) if name == "boxShadow" => {
            let mut shadows = Vec::new();
            for item in items {
                shadows.push(crate::scene::BoxShadow {
                    offset: pair(required(item, "offset")?)?,
                    blur_sigma: required(item, "blurSigma")?.number(name)?,
                    spread: item
                        .field("spread")
                        .map(|v| v.number(name))
                        .transpose()?
                        .unwrap_or(0.0),
                    color: effect_color(required(item, "color")?)?,
                });
            }
            set(Property::BoxShadow(shadows))
        }
        Value::Array(items) if is_color_name(name) => {
            if items.len() != 4 {
                return Err(invalid(name, "RGBA requires four channels"));
            }
            let mut rgba = [0; 4];
            for (out, item) in rgba.iter_mut().zip(items) {
                let value = item.number(name)?;
                if !(0.0..=255.0).contains(&value) || value.fract() != 0.0 {
                    return Err(invalid(name, "RGBA channel must be an integer in 0..255"));
                }
                *out = u8::try_from(whole_u16(name, value)?)
                    .map_err(|_| invalid(name, "RGBA channel out of range"))?;
            }
            color_value(name, PaintColor::rgba(rgba))
        }
        Value::Array(items)
            if matches!(
                name,
                "gridTemplateColumns" | "gridTemplateRows" | "gridAutoColumns" | "gridAutoRows"
            ) =>
        {
            let mut tracks = Vec::new();
            for item in items {
                tracks.push(
                    if let (Some(min), Some(max)) = (item.field("min"), item.field("max")) {
                        GridTrack::MinMax {
                            min: track_bound(min)?,
                            max: track_bound(max)?,
                        }
                    } else {
                        GridTrack::Bound(track_bound(item)?)
                    },
                );
            }
            track_property(name, TrackList::tracks(tracks))
        }
        Value::Object(_) if name == "shadow" => set(Property::Shadow(crate::scene::Shadow {
            color: effect_color(required(value, "color")?)?,
            elevation: required(value, "elevation")?.number(name)?,
        })),
        Value::Object(_) if name == "gradient" => set(Property::Gradient(gradient(value)?)),
        Value::Object(fields)
            if fields.len() == 1 && matches!(value.field("inherit"), Some(Value::Bool(true))) =>
        {
            Ok(vec![Declaration::inherit(key_of(name)?)])
        }
        Value::Object(_) if is_color_name(name) => {
            let Some(Value::String(token)) = value.field("token") else {
                return Err(invalid(name, "expected a color token"));
            };
            let color = if let Some(opacity) = value.field("opacity") {
                let opacity = opacity.number(name)?;
                if !(0.0..=1.0).contains(&opacity) {
                    return Err(invalid(name, "opacity must be in 0..1"));
                }
                PaintColor::TokenAlpha(
                    std::sync::Arc::from(token.as_str()),
                    u8::try_from(whole_u16(name, (opacity * 255.0).round())?)
                        .map_err(|_| invalid(name, "opacity out of range"))?,
                )
            } else {
                PaintColor::token(token)
            };
            color_value(name, color)
        }
        Value::Object(_)
            if matches!(
                name,
                "gridColumnStart" | "gridColumnEnd" | "gridRowStart" | "gridRowEnd"
            ) =>
        {
            let place = if let Some(line) = value.field("line") {
                let number = line.number(name)?;
                if number.fract() != 0.0
                    || number == 0.0
                    || number < f32::from(i16::MIN)
                    || number > f32::from(i16::MAX)
                {
                    return Err(invalid(name, "invalid grid line"));
                }
                GridPlacement::Line(
                    i16::try_from(whole_i32(name, number)?)
                        .map_err(|_| invalid(name, "grid line out of range"))?,
                )
            } else if let Some(span) = value.field("span") {
                let number = span.number(name)?;
                if number.fract() != 0.0 || number < 1.0 || number > f32::from(u16::MAX) {
                    return Err(invalid(name, "invalid grid span"));
                }
                GridPlacement::Span(whole_u16(name, number)?)
            } else {
                return Err(invalid(name, "expected a grid line or span"));
            };
            set(match name {
                "gridColumnStart" => Property::GridColumnStart(place),
                "gridColumnEnd" => Property::GridColumnEnd(place),
                "gridRowStart" => Property::GridRowStart(place),
                _ => Property::GridRowEnd(place),
            })
        }
        Value::Object(_) if name == "lineHeight" => {
            let points = value
                .field("points")
                .ok_or_else(|| invalid(name, "expected points"))?
                .number(name)?;
            set(Property::LineHeight(LineHeightValue::Points(points)))
        }
        Value::Object(_)
            if matches!(name, "width" | "height" | "flexBasis")
                && value.field("fitContent").is_some() =>
        {
            let length = object_length(
                value
                    .field("fitContent")
                    .ok_or_else(|| invalid(name, "expected fitContent length"))?,
            )?;
            set(size_word(name, SizeValue::FitContentLimit(length))?)
        }
        Value::Object(_) => {
            if let Some(Value::String(token)) = value.field("token") {
                return token_value(name, token);
            }
            env_length(name, object_length(value)?)
        }
        _ => Err(invalid(name, "unsupported style value")),
    }
}

/// Numeric wire selector for one compiler-generated inline style patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScalarStyleOp(u8);

const SCALAR_STYLES: &[(&str, &[Key])] = &[
    ("opacity", &[Key::Opacity]),
    ("translateX", &[Key::TranslateX]),
    ("translateY", &[Key::TranslateY]),
    ("scale", &[Key::ScaleX, Key::ScaleY]),
    ("scaleX", &[Key::ScaleX]),
    ("scaleY", &[Key::ScaleY]),
    ("rotate", &[Key::Rotate]),
    ("width", &[Key::Width]),
    ("height", &[Key::Height]),
    ("borderRadius", &[Key::BorderRadius]),
    ("zIndex", &[Key::ZIndex]),
    ("backgroundColor", &[Key::BackgroundColor]),
    ("color", &[Key::Color]),
    ("borderColor", &[Key::BorderColor]),
];

impl ScalarStyleOp {
    /// Select a fast path for a known JSX style property.
    pub fn parse(name: &str) -> Option<Self> {
        let index = SCALAR_STYLES.iter().position(|(key, _)| *key == name)?;
        Self::from_code(u8::try_from(index).ok()?)
    }

    /// Validate the decoded byte before using it as a selector.
    pub fn from_code(code: u8) -> Option<Self> {
        SCALAR_STYLES.get(usize::from(code)).map(|_| Self(code))
    }

    /// The byte written into an inline patch call.
    pub const fn code(self) -> u8 {
        self.0
    }

    #[cfg(feature = "js")]
    pub(crate) fn name(self) -> &'static str {
        SCALAR_STYLES[usize::from(self.0)].0
    }

    #[cfg(feature = "js")]
    pub(crate) fn keys(self) -> &'static [Key] {
        SCALAR_STYLES[usize::from(self.0)].1
    }

    #[cfg(feature = "js")]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "packed color range and integrality are checked"
    )]
    pub(crate) fn number(self, value: f64) -> Result<Vec<Declaration>, StyleError> {
        if self.0 >= 11 {
            if !(0. ..=f64::from(u32::MAX)).contains(&value) || value.fract() != 0. {
                return Err(invalid(self.name(), "expected a packed RGBA integer"));
            }
            return color_value(self.name(), PaintColor::Rgba((value as u32).to_be_bytes()));
        }
        let value = finite_f32(self.name(), value)?;
        let points = Length::points(value);
        set(match self.0 {
            0 => Property::Opacity(value),
            1 => Property::TranslateX(points),
            2 => Property::TranslateY(points),
            3 => return scale(value),
            4 => Property::ScaleX(value),
            5 => Property::ScaleY(value),
            6 => Property::Rotate(value),
            7 => Property::Width(SizeValue::Length(points)),
            8 => Property::Height(SizeValue::Length(points)),
            9 => Property::BorderRadius(points),
            10 => Property::ZIndex(whole_i32(self.name(), value)?),
            _ => return Err(unknown(self.name())),
        })
    }
}

fn scale(value: f32) -> Result<Vec<Declaration>, StyleError> {
    let x = Property::ScaleX(value);
    let y = Property::ScaleY(value);
    x.validate()?;
    y.validate()?;
    Ok(vec![Declaration::set(x), Declaration::set(y)])
}

fn object_length(value: &Value) -> Result<Length, StyleError> {
    match value {
        Value::Number(value) => Ok(Length::points(finite_f32("length", *value)?)),
        Value::String(value) if value.ends_with('%') => value[..value.len() - 1]
            .parse::<f32>()
            .map(|n| Length::fraction(n / 100.0))
            .map_err(|_| invalid("length", "bad percentage")),
        Value::Object(fields) if fields.len() == 1 => {
            let (key, value) = &fields[0];
            if let ("token", Value::String(name)) = (key.as_str(), value) {
                return Ok(Length::token(name));
            }
            let number = value.number("length")?;
            Ok(match key.as_str() {
                "rem" => Length::rem(number),
                "vw" => Length::ViewportWidth(number),
                "vh" => Length::ViewportHeight(number),
                "steps" => Length::steps(number),
                _ => return Err(invalid("length", "unknown length unit")),
            })
        }
        _ => Err(invalid("length", "expected a length")),
    }
}
fn track_bound(value: &Value) -> Result<super::TrackBound, StyleError> {
    use super::TrackBound as B;
    Ok(match value {
        Value::String(word) => match word.as_str() {
            "auto" => B::Auto,
            "min-content" => B::MinContent,
            "max-content" => B::MaxContent,
            _ => return Err(invalid("grid", "unknown track keyword")),
        },
        Value::Object(fields) if fields.len() == 1 => {
            let (key, value) = &fields[0];
            let number = value.number("grid")?;
            match key.as_str() {
                "points" => B::Points(number),
                "fr" => B::Fraction(number),
                "percent" => B::Percent(number),
                _ => return Err(invalid("grid", "unknown track unit")),
            }
        }
        _ => return Err(invalid("grid", "expected a track bound")),
    })
}
fn track_property(name: &str, tracks: TrackList) -> Result<Vec<Declaration>, StyleError> {
    set(match name {
        "gridTemplateColumns" => Property::GridTemplateColumns(tracks),
        "gridTemplateRows" => Property::GridTemplateRows(tracks),
        "gridAutoColumns" => Property::GridAutoColumns(tracks),
        "gridAutoRows" => Property::GridAutoRows(tracks),
        _ => return Err(unknown(name)),
    })
}

fn finite_f32(name: &str, number: f64) -> Result<f32, StyleError> {
    if !number.is_finite() || number > f64::from(f32::MAX) || number < f64::from(f32::MIN) {
        return Err(invalid(name, "number is out of range"));
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the range check keeps the value inside f32"
    )]
    Ok(number as f32)
}

fn whole_i32(name: &str, value: f32) -> Result<i32, StyleError> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < -2_147_483_648.0
        || value > 2_147_483_647.0
    {
        return Err(invalid(name, "number is out of range"));
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the range check keeps the value inside i32"
    )]
    Ok(value as i32)
}

fn whole_u16(name: &str, value: f32) -> Result<u16, StyleError> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f32::from(u16::MAX) {
        return Err(invalid(name, "number is out of range"));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the range check keeps the value inside u16"
    )]
    Ok(value as u16)
}

fn whole_u32(name: &str, value: f32) -> Result<u32, StyleError> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value >= 4_294_967_296.0 {
        return Err(invalid(name, "number is out of range"));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the range check keeps the value inside u32"
    )]
    Ok(value as u32)
}

fn is_shorthand(name: &str) -> bool {
    matches!(name, "padding" | "margin" | "gap" | "flex" | "inset")
}

fn resets(name: &str) -> Result<Vec<Declaration>, StyleError> {
    let keys: &[Key] = match name {
        "padding" => &[
            Key::PaddingTop,
            Key::PaddingRight,
            Key::PaddingBottom,
            Key::PaddingLeft,
        ],
        "margin" => &[
            Key::MarginTop,
            Key::MarginRight,
            Key::MarginBottom,
            Key::MarginLeft,
        ],
        "gap" => &[Key::RowGap, Key::ColumnGap],
        "flex" => &[Key::FlexGrow, Key::FlexShrink, Key::FlexBasis],
        "inset" => &[Key::Top, Key::Right, Key::Bottom, Key::Left],
        "scale" => &[Key::ScaleX, Key::ScaleY],
        _ => return Ok(vec![Declaration::reset(key_of(name)?)]),
    };
    Ok(keys.iter().copied().map(Declaration::reset).collect())
}

fn key_of(name: &str) -> Result<Key, StyleError> {
    if name == "border" {
        return Ok(Key::BorderWidth);
    }
    Key::parse(name).ok_or_else(|| unknown(name))
}

fn number_value(name: &str, value: f32) -> Result<Vec<Declaration>, StyleError> {
    if !value.is_finite() {
        return Err(invalid(name, "number is not finite"));
    }
    match name {
        "padding" => expand_padding(Length::points(value)),
        "margin" => expand_margin(MarginValue::Length(Length::points(value))),
        "gap" => expand_gap(Length::points(value)),
        "inset" => expand_inset(InsetValue::Length(Length::points(value))),
        "flex" => expand_flex(FlexShorthand::Number(value)),
        "flexGrow" => set(Property::FlexGrow(value)),
        "flexShrink" => set(Property::FlexShrink(value)),
        "opacity" => set(Property::Opacity(value)),
        "zIndex" => set(Property::ZIndex(whole_i32(name, value)?)),
        "rotate" => set(Property::Rotate(value)),
        "scale" => scale(value),
        "scaleX" => set(Property::ScaleX(value)),
        "scaleY" => set(Property::ScaleY(value)),
        "aspectRatio" => set(Property::AspectRatio(value)),
        "fontWeight" => set(Property::FontWeight(whole_u16(name, value)?)),
        "maxLines" => set(Property::MaxLines(whole_u32(name, value)?)),
        "lineHeight" => set(Property::LineHeight(LineHeightValue::Factor(value))),
        "softWrap" => set(Property::SoftWrap(value > 0.5)),
        "blur" => set(Property::Blur(value)),
        "backdrop" => set(Property::Backdrop(value)),
        _ => length_declarations(name, Length::points(value)),
    }
}

fn string_value(name: &str, payload: &str) -> Result<Vec<Declaration>, StyleError> {
    if name == "flex" {
        return expand_flex(match payload {
            "auto" => FlexShorthand::Auto,
            "none" => FlexShorthand::None,
            "initial" => FlexShorthand::Initial,
            _ => return Err(invalid(name, "unsupported flex keyword")),
        });
    }
    if name == "margin" && payload == "auto" {
        return expand_margin(MarginValue::Auto);
    }
    if let Some(percent) = payload.strip_suffix('%') {
        let value: f32 = percent
            .parse()
            .map_err(|_| invalid(name, "bad percentage"))?;
        return length_declarations(name, Length::fraction(value / 100.0));
    }
    if payload == "auto" {
        use super::GridPlacement as G;
        let property = match name {
            "marginTop" => Some(Property::MarginTop(MarginValue::Auto)),
            "marginRight" => Some(Property::MarginRight(MarginValue::Auto)),
            "marginBottom" => Some(Property::MarginBottom(MarginValue::Auto)),
            "marginLeft" => Some(Property::MarginLeft(MarginValue::Auto)),
            "marginInlineStart" => Some(Property::MarginInlineStart(MarginValue::Auto)),
            "marginInlineEnd" => Some(Property::MarginInlineEnd(MarginValue::Auto)),
            "top" => Some(Property::Top(InsetValue::Auto)),
            "right" => Some(Property::Right(InsetValue::Auto)),
            "bottom" => Some(Property::Bottom(InsetValue::Auto)),
            "left" => Some(Property::Left(InsetValue::Auto)),
            "insetInlineStart" => Some(Property::InsetInlineStart(InsetValue::Auto)),
            "insetInlineEnd" => Some(Property::InsetInlineEnd(InsetValue::Auto)),
            "gridColumnStart" => Some(Property::GridColumnStart(G::Auto)),
            "gridColumnEnd" => Some(Property::GridColumnEnd(G::Auto)),
            "gridRowStart" => Some(Property::GridRowStart(G::Auto)),
            "gridRowEnd" => Some(Property::GridRowEnd(G::Auto)),
            _ => None,
        };
        if let Some(property) = property {
            return set(property);
        }
        if name == "inset" {
            return expand_inset(InsetValue::Auto);
        }
    }
    if name == "gridAutoFlow" {
        return set(Property::GridAutoFlow(match payload {
            "row" => super::GridFlow::Row,
            "column" => super::GridFlow::Column,
            "row dense" => super::GridFlow::RowDense,
            "column dense" => super::GridFlow::ColumnDense,
            _ => return Err(invalid(name, "invalid grid flow")),
        }));
    }
    if payload.starts_with('#') {
        return color_value(name, PaintColor::rgba(parse_hex(payload)?));
    }
    if let Some(property) = keyword(name, payload)? {
        return set(property);
    }
    match name {
        "fontFamily" => set(Property::FontFamily(payload.to_owned())),
        "ellipsis" => set(Property::Ellipsis(payload.to_owned())),
        "gridTemplateColumns" => set(Property::GridTemplateColumns(TrackList::source(payload))),
        "gridTemplateRows" => set(Property::GridTemplateRows(TrackList::source(payload))),
        "gridAutoColumns" | "gridAutoRows" => track_property(name, TrackList::source(payload)),
        _ => Err(invalid(name, "unsupported string")),
    }
}

fn env_length(name: &str, length: Length) -> Result<Vec<Declaration>, StyleError> {
    if name == "fontSize" {
        let absolute = match &length {
            Length::Rem(value) => AbsoluteLength::Rem(*value),
            Length::ViewportWidth(value) => AbsoluteLength::ViewportWidth(*value),
            Length::ViewportHeight(value) => AbsoluteLength::ViewportHeight(*value),
            Length::Points(value) => AbsoluteLength::points(*value),
            Length::Token(token) => AbsoluteLength::Token(token.clone()),
            Length::Fraction(_) | Length::Steps(_) => {
                return Err(invalid(name, "fontSize cannot be a percentage"));
            }
        };
        return set(Property::FontSize(absolute));
    }
    length_declarations(name, length)
}

fn token_value(name: &str, payload: &str) -> Result<Vec<Declaration>, StyleError> {
    if payload.is_empty() {
        return Err(invalid(name, "missing token"));
    }
    if is_color_name(name) {
        return color_value(name, PaintColor::token(payload));
    }
    env_length(name, Length::token(payload))
}

fn color_value(name: &str, color: PaintColor) -> Result<Vec<Declaration>, StyleError> {
    set(match name {
        "backgroundColor" => Property::BackgroundColor(color),
        "borderColor" => Property::BorderColor(color),
        "outlineColor" => Property::OutlineColor(color),
        "color" => Property::Color(color),
        _ => return Err(unknown(name)),
    })
}

fn keyword(name: &str, word: &str) -> Result<Option<Property>, StyleError> {
    let property = match (name, word) {
        ("display", "flex") => Property::Display(DisplayMode::Flex),
        ("display", "grid") => Property::Display(DisplayMode::Grid),
        ("display", "block") => Property::Display(DisplayMode::Block),
        ("display", "flow-root") => Property::Display(DisplayMode::FlowRoot),
        ("display", "none") => Property::Display(DisplayMode::None),
        ("flexDirection", "row") => Property::FlexDirection(FlexDir::Row),
        ("flexDirection", "column") => Property::FlexDirection(FlexDir::Column),
        ("flexDirection", "row-reverse") => Property::FlexDirection(FlexDir::RowReverse),
        ("flexDirection", "column-reverse") => Property::FlexDirection(FlexDir::ColumnReverse),
        ("flexWrap", "nowrap") => Property::FlexWrap(FlexWrapMode::NoWrap),
        ("flexWrap", "wrap") => Property::FlexWrap(FlexWrapMode::Wrap),
        ("flexWrap", "wrap-reverse") => Property::FlexWrap(FlexWrapMode::WrapReverse),
        ("width" | "height" | "flexBasis", "auto") => {
            return Ok(Some(size_word(name, SizeValue::Auto)?));
        }
        ("width" | "height" | "flexBasis", "min-content") => {
            return Ok(Some(size_word(name, SizeValue::MinContent)?));
        }
        ("width" | "height" | "flexBasis", "max-content") => {
            return Ok(Some(size_word(name, SizeValue::MaxContent)?));
        }
        ("width" | "height" | "flexBasis", "fit-content") => {
            return Ok(Some(size_word(name, SizeValue::FitContent)?));
        }
        ("width" | "height", "stretch") => return Ok(Some(size_word(name, SizeValue::Stretch)?)),
        ("lineHeight", "normal") => Property::LineHeight(LineHeightValue::Normal),
        ("blend", "src-over") => Property::Blend(crate::scene::BlendMode::SrcOver),
        ("blend", "multiply") => Property::Blend(crate::scene::BlendMode::Multiply),
        ("blend", "screen") => Property::Blend(crate::scene::BlendMode::Screen),
        ("cursor", value) => {
            let Some(cursor) = super::CursorStyle::parse(value) else {
                return Ok(None);
            };
            Property::Cursor(cursor)
        }
        ("objectFit", "fill") => Property::ObjectFit(crate::scene::ContentMode::Fill),
        ("objectFit", "contain") => Property::ObjectFit(crate::scene::ContentMode::Fit),
        ("objectFit", "cover") => Property::ObjectFit(crate::scene::ContentMode::Crop),
        ("flexBasis", "content") => Property::FlexBasis(BasisValue::Content),
        ("minWidth" | "minHeight", "auto") | ("maxWidth" | "maxHeight", "none") => {
            return Ok(Some(limit_word(name)?));
        }
        ("overflow" | "overflowX" | "overflowY", "visible") => {
            return Ok(Some(overflow_word(name, OverflowMode::Visible)?));
        }
        ("overflow" | "overflowX" | "overflowY", "clip") => {
            return Ok(Some(overflow_word(name, OverflowMode::Clip)?));
        }
        ("overflow" | "overflowX" | "overflowY", "hidden") => {
            return Ok(Some(overflow_word(name, OverflowMode::Hidden)?));
        }
        ("boxSizing", "border-box") => Property::BoxSizing(super::BoxSizingMode::BorderBox),
        ("boxSizing", "content-box") => Property::BoxSizing(super::BoxSizingMode::ContentBox),
        ("position", "relative") => Property::Position(PositionMode::Relative),
        ("position", "absolute") => Property::Position(PositionMode::Absolute),
        ("layoutDirection" | "writingDirection", "ltr") => {
            return Ok(Some(direction_word(name, false)?));
        }
        ("layoutDirection" | "writingDirection", "rtl") => {
            return Ok(Some(direction_word(name, true)?));
        }
        ("alignItems" | "alignSelf" | "justifyItems" | "justifySelf", _) => {
            return Ok(Some(item_word(name, word)?));
        }
        ("justifyContent" | "alignContent", _) => return Ok(Some(content_word(name, word)?)),
        ("textOverflow", "clip") => Property::TextOverflow(crate::scene::TextOverflow::Clip),
        ("textOverflow", "ellipsis") => {
            Property::TextOverflow(crate::scene::TextOverflow::Ellipsis)
        }
        ("textOverflow", "visible") => Property::TextOverflow(crate::scene::TextOverflow::Visible),
        ("textAlign", "left") => Property::TextAlign(crate::scene::TextAlign::Left),
        ("textAlign", "center") => Property::TextAlign(crate::scene::TextAlign::Center),
        ("textAlign", "right") => Property::TextAlign(crate::scene::TextAlign::Right),
        _ => return Ok(None),
    };
    Ok(Some(property))
}

fn size_word(name: &str, value: SizeValue) -> Result<Property, StyleError> {
    Ok(match name {
        "width" => Property::Width(value),
        "height" => Property::Height(value),
        "flexBasis" => Property::FlexBasis(match value {
            SizeValue::Auto => BasisValue::Size(SizeValue::Auto),
            other => BasisValue::Size(other),
        }),
        _ => return Err(unknown(name)),
    })
}

fn limit_word(name: &str) -> Result<Property, StyleError> {
    let value = LimitValue::Auto;
    Ok(match name {
        "minWidth" => Property::MinWidth(value),
        "minHeight" => Property::MinHeight(value),
        "maxWidth" => Property::MaxWidth(value),
        "maxHeight" => Property::MaxHeight(value),
        _ => return Err(unknown(name)),
    })
}

fn overflow_word(name: &str, mode: OverflowMode) -> Result<Property, StyleError> {
    Ok(match name {
        "overflow" => Property::Overflow(mode),
        "overflowX" => Property::OverflowX(mode),
        "overflowY" => Property::OverflowY(mode),
        _ => return Err(unknown(name)),
    })
}

fn direction_word(name: &str, rtl: bool) -> Result<Property, StyleError> {
    Ok(match name {
        "layoutDirection" => Property::LayoutDirection(if rtl {
            DirectionMode::Rtl
        } else {
            DirectionMode::Ltr
        }),
        "writingDirection" => Property::WritingDirectionRtl(rtl),
        _ => return Err(unknown(name)),
    })
}

fn item_word(name: &str, word: &str) -> Result<Property, StyleError> {
    let align = match word {
        "normal" => ItemAlign::Normal,
        "start" => ItemAlign::Start,
        "end" => ItemAlign::End,
        "flex-start" => ItemAlign::FlexStart,
        "flex-end" => ItemAlign::FlexEnd,
        "center" => ItemAlign::Center,
        "stretch" => ItemAlign::Stretch,
        "baseline" => ItemAlign::Baseline,
        _ => return Err(invalid(name, "unsupported alignment")),
    };
    Ok(match name {
        "alignItems" => Property::AlignItems(align),
        "alignSelf" => Property::AlignSelf(align),
        "justifyItems" => Property::JustifyItems(align),
        "justifySelf" => Property::JustifySelf(align),
        _ => return Err(unknown(name)),
    })
}

fn content_word(name: &str, word: &str) -> Result<Property, StyleError> {
    let align = match word {
        "start" => ContentAlign::Start,
        "flex-start" => ContentAlign::FlexStart,
        "end" => ContentAlign::End,
        "flex-end" => ContentAlign::FlexEnd,
        "center" => ContentAlign::Center,
        "stretch" => ContentAlign::Stretch,
        "space-between" => ContentAlign::SpaceBetween,
        "space-around" => ContentAlign::SpaceAround,
        "space-evenly" => ContentAlign::SpaceEvenly,
        _ => return Err(invalid(name, "unsupported alignment")),
    };
    Ok(match name {
        "justifyContent" => Property::JustifyContent(align),
        "alignContent" => Property::AlignContent(align),
        _ => return Err(unknown(name)),
    })
}

fn length_declarations(name: &str, length: Length) -> Result<Vec<Declaration>, StyleError> {
    match name {
        "padding" => expand_padding(length),
        "margin" => expand_margin(MarginValue::Length(length)),
        "gap" => expand_gap(length),
        "inset" => expand_inset(InsetValue::Length(length)),
        "border" => set(Property::BorderWidth(length)),
        _ => set(length_property(name, length)?),
    }
}

fn length_property(name: &str, length: Length) -> Result<Property, StyleError> {
    if name == "fontSize" {
        return env_length(name, length).and_then(|items| match items.into_iter().next() {
            Some(Declaration::Set(property)) => Ok(property),
            _ => Err(unknown(name)),
        });
    }
    Ok(match name {
        "width" => Property::Width(SizeValue::Length(length)),
        "height" => Property::Height(SizeValue::Length(length)),
        "minWidth" => Property::MinWidth(LimitValue::Length(length)),
        "minHeight" => Property::MinHeight(LimitValue::Length(length)),
        "maxWidth" => Property::MaxWidth(LimitValue::Length(length)),
        "maxHeight" => Property::MaxHeight(LimitValue::Length(length)),
        "flexBasis" => Property::FlexBasis(BasisValue::Size(SizeValue::Length(length))),
        "paddingTop" => Property::PaddingTop(length),
        "paddingRight" => Property::PaddingRight(length),
        "paddingBottom" => Property::PaddingBottom(length),
        "paddingLeft" => Property::PaddingLeft(length),
        "paddingInlineStart" => Property::PaddingInlineStart(length),
        "paddingInlineEnd" => Property::PaddingInlineEnd(length),
        "marginTop" => Property::MarginTop(MarginValue::Length(length)),
        "marginRight" => Property::MarginRight(MarginValue::Length(length)),
        "marginBottom" => Property::MarginBottom(MarginValue::Length(length)),
        "marginLeft" => Property::MarginLeft(MarginValue::Length(length)),
        "marginInlineStart" => Property::MarginInlineStart(MarginValue::Length(length)),
        "marginInlineEnd" => Property::MarginInlineEnd(MarginValue::Length(length)),
        "rowGap" => Property::RowGap(length),
        "columnGap" => Property::ColumnGap(length),
        "top" => Property::Top(InsetValue::Length(length)),
        "right" => Property::Right(InsetValue::Length(length)),
        "bottom" => Property::Bottom(InsetValue::Length(length)),
        "left" => Property::Left(InsetValue::Length(length)),
        "insetInlineStart" => Property::InsetInlineStart(InsetValue::Length(length)),
        "insetInlineEnd" => Property::InsetInlineEnd(InsetValue::Length(length)),
        "borderWidth" => Property::BorderWidth(length),
        "borderRadius" => Property::BorderRadius(length),
        "outlineWidth" => Property::OutlineWidth(length),
        "translateX" => Property::TranslateX(length),
        "translateY" => Property::TranslateY(length),
        "transformOriginX" => Property::TransformOriginX(length),
        "transformOriginY" => Property::TransformOriginY(length),
        _ => return Err(unknown(name)),
    })
}

fn is_color_name(name: &str) -> bool {
    matches!(
        name,
        "color" | "backgroundColor" | "borderColor" | "outlineColor"
    )
}

fn set(property: Property) -> Result<Vec<Declaration>, StyleError> {
    property.validate()?;
    Ok(vec![Declaration::set(property)])
}

fn parse_hex(text: &str) -> Result<[u8; 4], StyleError> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 && hex.len() != 8 {
        return Err(invalid("color", "color must be #RRGGBB or #RRGGBBAA"));
    }
    let mut out = [0, 0, 0, 255];
    let bytes = hex.as_bytes();
    for index in (0..hex.len()).step_by(2) {
        let high = hex_digit(bytes[index])?;
        let low = hex_digit(bytes[index + 1])?;
        out[index / 2] = (high << 4) | low;
    }
    Ok(out)
}

fn hex_digit(byte: u8) -> Result<u8, StyleError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid("color", "color must be hex")),
    }
}

fn invalid(name: &str, reason: &'static str) -> StyleError {
    StyleError::Invalid {
        node: None,
        property: name_static(name),
        reason,
    }
}

fn unknown(name: &str) -> StyleError {
    StyleError::UnknownProperty {
        node: None,
        name: name.to_owned(),
    }
}

fn name_static(name: &str) -> &'static str {
    Key::parse(name).map(Key::name).unwrap_or("style")
}

fn effect_color(value: &Value) -> Result<[u8; 4], StyleError> {
    match declarations("color", value)?.into_iter().next() {
        Some(Declaration::Set(Property::Color(PaintColor::Rgba(color)))) => Ok(color),
        _ => Err(invalid(
            "color",
            "effect colors must be literal RGBA or hex",
        )),
    }
}
fn required<'a>(value: &'a Value, name: &str) -> Result<&'a Value, StyleError> {
    value
        .field(name)
        .ok_or_else(|| invalid("effect", "missing effect field"))
}
fn pair(value: &Value) -> Result<[f32; 2], StyleError> {
    let Value::Array(items) = value else {
        return Err(invalid("effect", "expected two coordinates"));
    };
    if items.len() != 2 {
        return Err(invalid("effect", "expected two coordinates"));
    }
    Ok([items[0].number("effect")?, items[1].number("effect")?])
}
fn gradient(value: &Value) -> Result<crate::scene::Gradient, StyleError> {
    let Value::Array(items) = required(value, "stops")? else {
        return Err(invalid("gradient", "expected color stops"));
    };
    let mut stops = Vec::new();
    for stop in items {
        stops.push(crate::scene::ColorStop::srgb(
            required(stop, "offset")?.number("gradient")?,
            effect_color(required(stop, "color")?)?,
        ));
    }
    let center = value
        .field("center")
        .map(pair)
        .transpose()?
        .unwrap_or([0.5, 0.5]);
    Ok(match value.field("kind") {
        Some(Value::String(kind)) if kind == "radial" => crate::scene::Gradient::Radial {
            center,
            radius: required(value, "radius")?.number("gradient")?,
            stops,
        },
        Some(Value::String(kind)) if kind == "sweep" => crate::scene::Gradient::Sweep {
            center,
            start_angle_rad: required(value, "startAngle")?.number("gradient")?,
            end_angle_rad: required(value, "endAngle")?.number("gradient")?,
            stops,
        },
        None | Some(Value::String(_)) => {
            if let Some(Value::String(kind)) = value.field("kind")
                && kind != "linear"
            {
                return Err(invalid("gradient", "unknown gradient kind"));
            }
            crate::scene::Gradient::Linear {
                start: pair(required(value, "from")?)?,
                end: pair(required(value, "to")?)?,
                stops,
            }
        }
        _ => return Err(invalid("gradient", "expected gradient kind")),
    })
}
