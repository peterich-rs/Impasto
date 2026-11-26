//! Typed style declarations. Layout, paint, and text share one property set.
//!
//! A [`Style`] is a snapshot of one source layer. [`Patch`] merges into the
//! imperative layer by key. Shorthands expand before they are stored. Missing
//! keys are not declarations. [`Declaration::Reset`] blocks lower sources and
//! inheritance. [`Declaration::Inherit`] asks the parent for that key.

mod classes;
pub mod codec;
pub(crate) mod defaults;
mod environment;
mod property;
mod resolve;
#[cfg(feature = "style-binary")]
pub mod static_pool;
pub(crate) mod taffy;
mod values;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use thiserror::Error;

pub use classes::{ClassCache, ParsedClass, parse_class};
#[cfg(feature = "style-binary")]
pub use classes::{StaticClass, decode_static_class, encode_static_class};
pub use environment::{
    BREAKPOINTS, CLASS_BYTE_LIMIT, DepMask, Environment, FontSizes, Interaction, Radii, TypeRole,
};
pub use property::{
    FieldMeta, Impact, Key, Property, display_flex, expand_flex, expand_gap, expand_inset,
    expand_margin, expand_padding, field_metas,
};
pub use resolve::{Resolved, WinningDeclaration, merge_layers};

/// Where a declaration layer sits. Lower origins lose to higher ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StyleOrigin {
    Component,
    Classes,
    Inline,
    Imperative,
    Animation,
}

impl StyleOrigin {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::Classes => "classes",
            Self::Inline => "inline",
            Self::Imperative => "imperative",
            Self::Animation => "animation",
        }
    }
}

/// Which author declarations a node may accept.
///
/// Stack and scroll viewports take their surface from the node kind. List
/// content is registered on the driver's content node. Component sources keep
/// structural presets; class, inline, imperative, and animation do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StyleSurface {
    #[default]
    General,
    Stack,
    ScrollViewport,
    ListContent,
}

impl StyleSurface {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "general" => Some(Self::General),
            "stack" => Some(Self::Stack),
            "scroll-viewport" => Some(Self::ScrollViewport),
            "list-content" => Some(Self::ListContent),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Stack => "stack",
            Self::ScrollViewport => "scroll-viewport",
            Self::ListContent => "list-content",
        }
    }
}

fn track_key(key: Key) -> bool {
    matches!(
        key,
        Key::GridTemplateColumns
            | Key::GridTemplateRows
            | Key::GridAutoColumns
            | Key::GridAutoRows
            | Key::GridAutoFlow
    )
}

fn overflow_key(key: Key) -> bool {
    matches!(key, Key::Overflow | Key::OverflowX | Key::OverflowY)
}

fn list_content_key(key: Key) -> bool {
    if matches!(
        key,
        Key::Color
            | Key::FontFamily
            | Key::FontSize
            | Key::FontWeight
            | Key::LineHeight
            | Key::TextAlign
            | Key::WritingDirection
    ) {
        return true;
    }
    let impact = key.impact();
    impact.contains(Impact::PAINT) && !impact.intersects(Impact::LAYOUT | Impact::MEASURE)
}

/// Author declaration on `surface`. `display: none` stays available.
pub fn author_key_allowed(surface: StyleSurface, declaration: &Declaration) -> bool {
    if matches!(
        declaration,
        Declaration::Set(Property::Display(DisplayMode::None))
    ) {
        return true;
    }
    let key = declaration.key();
    match surface {
        StyleSurface::General => true,
        StyleSurface::Stack => key != Key::Display && !track_key(key),
        StyleSurface::ScrollViewport => {
            key != Key::Display && !track_key(key) && !overflow_key(key)
        }
        StyleSurface::ListContent => key != Key::Display && list_content_key(key),
    }
}

/// Handle for one extra source on a node. A stale generation does not write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleSourceId {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

impl StyleSourceId {
    pub fn to_raw(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.slot)
    }
    pub fn from_raw(raw: u64) -> Self {
        let bytes = raw.to_le_bytes();
        Self {
            slot: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            generation: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }
}

/// Environment reads of one declaration, independent of its source origin.
pub(crate) fn dependencies(property: &Property) -> DepMask {
    use Property as P;
    fn length(value: &Length) -> DepMask {
        match value {
            Length::Token(_) => DepMask::THEME,
            Length::Rem(_)
            | Length::Steps(_)
            | Length::ViewportWidth(_)
            | Length::ViewportHeight(_) => DepMask::WINDOW,
            _ => DepMask::empty(),
        }
    }
    fn size(value: &SizeValue) -> DepMask {
        match value {
            SizeValue::Length(value) | SizeValue::FitContentLimit(value) => length(value),
            _ => DepMask::empty(),
        }
    }
    match property {
        P::BackgroundColor(PaintColor::Token(_))
        | P::Color(PaintColor::Token(_))
        | P::BorderColor(PaintColor::Token(_))
        | P::OutlineColor(PaintColor::Token(_)) => DepMask::THEME,
        P::BackgroundColor(PaintColor::TokenAlpha(..))
        | P::Color(PaintColor::TokenAlpha(..))
        | P::BorderColor(PaintColor::TokenAlpha(..))
        | P::OutlineColor(PaintColor::TokenAlpha(..)) => DepMask::THEME,
        P::FontSize(value) => length(&value.as_length()),
        P::Width(value) | P::Height(value) => size(value),
        P::FlexBasis(BasisValue::Size(value)) => size(value),
        P::MinWidth(LimitValue::Length(value))
        | P::MinHeight(LimitValue::Length(value))
        | P::MaxWidth(LimitValue::Length(value))
        | P::MaxHeight(LimitValue::Length(value))
        | P::PaddingTop(value)
        | P::PaddingRight(value)
        | P::PaddingBottom(value)
        | P::PaddingLeft(value)
        | P::PaddingInlineStart(value)
        | P::PaddingInlineEnd(value)
        | P::MarginTop(MarginValue::Length(value))
        | P::MarginRight(MarginValue::Length(value))
        | P::MarginBottom(MarginValue::Length(value))
        | P::MarginLeft(MarginValue::Length(value))
        | P::MarginInlineStart(MarginValue::Length(value))
        | P::MarginInlineEnd(MarginValue::Length(value))
        | P::Top(InsetValue::Length(value))
        | P::Right(InsetValue::Length(value))
        | P::Bottom(InsetValue::Length(value))
        | P::Left(InsetValue::Length(value))
        | P::InsetInlineStart(InsetValue::Length(value))
        | P::InsetInlineEnd(InsetValue::Length(value))
        | P::RowGap(value)
        | P::ColumnGap(value)
        | P::BorderWidth(value)
        | P::BorderRadius(value)
        | P::OutlineWidth(value)
        | P::TranslateX(value)
        | P::TranslateY(value)
        | P::TransformOriginX(value)
        | P::TransformOriginY(value) => length(value),
        _ => DepMask::empty(),
    }
}
pub use values::{
    AbsoluteLength, BasisValue, BoxSizingMode, ContentAlign, DirectionMode, DisplayMode, FlexDir,
    FlexShorthand, FlexWrapMode, GridFlow, GridPlacement, GridTrack, InsetValue, ItemAlign, Length,
    LimitValue, LineHeightValue, MarginValue, OverflowMode, PaintColor, PositionMode, SizeValue,
    TrackBound, TrackList,
};

/// One stored declaration.
///
/// `Reset` blocks lower sources and inheritance. `Inherit` asks the parent
/// even when a lower source set the key. Omitting a key drops only this source.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "style-binary", derive(serde::Serialize, serde::Deserialize))]
pub enum Declaration {
    Set(Property),
    Reset(Key),
    Inherit(Key),
}

impl Declaration {
    pub fn set(property: Property) -> Self {
        Self::Set(property)
    }

    pub fn reset(key: Key) -> Self {
        Self::Reset(key)
    }

    pub fn inherit(key: Key) -> Self {
        Self::Inherit(key)
    }

    pub fn key(&self) -> Key {
        match self {
            Self::Set(property) => property.key(),
            Self::Reset(key) | Self::Inherit(key) => *key,
        }
    }

    pub fn validate(&self) -> Result<(), StyleError> {
        match self {
            Self::Set(property) => property.validate(),
            Self::Reset(_) | Self::Inherit(_) => Ok(()),
        }
    }
}

/// One source layer. Later fragments replace keys they contain and leave the rest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    fragments: std::sync::Arc<Vec<Vec<Declaration>>>,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.iter().all(|fragment| fragment.is_empty())
    }

    pub fn fragment(declarations: Vec<Declaration>) -> Result<Self, StyleError> {
        let mut style = Self::new();
        style.push_fragment(declarations)?;
        Ok(style)
    }

    pub fn push_fragment(&mut self, declarations: Vec<Declaration>) -> Result<(), StyleError> {
        for declaration in &declarations {
            declaration.validate()?;
        }
        if !declarations.is_empty() {
            std::sync::Arc::make_mut(&mut self.fragments).push(declarations);
        }
        Ok(())
    }

    pub fn fragments(&self) -> &[Vec<Declaration>] {
        &self.fragments
    }

    /// Replace this layer with `other`. Used by `set_style`.
    pub fn replace(&mut self, other: Style) {
        *self = other;
    }
}

/// A partial write into the imperative layer. Keys not listed stay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    declarations: Vec<Declaration>,
}

impl Patch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, property: Property) -> Result<(), StyleError> {
        property.validate()?;
        self.declarations.push(Declaration::Set(property));
        Ok(())
    }

    pub fn reset(&mut self, key: Key) {
        self.declarations.push(Declaration::Reset(key));
    }

    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }

    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }
}

/// Declarations kept per node. Empty nodes do not allocate this.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    pub defaults: Style,
    pub class_text: String,
    pub class_list: String,
    pub inline: Style,
    pub imperative: Style,
}

impl Sources {
    pub fn class_source(&self) -> String {
        let mut combined = self.class_text.clone();
        if !self.class_list.is_empty() {
            if !combined.is_empty() {
                combined.push(' ');
            }
            combined.push_str(&self.class_list);
        }
        combined
    }
}

/// Winning longhands after weak rules inside one layer have been resolved.
pub type LayerMap = BTreeMap<Key, Declaration>;

/// Flatten one layer. Later declarations of the same key replace earlier ones.
pub fn flatten(style: &Style) -> LayerMap {
    let mut map = LayerMap::new();
    for fragment in style.fragments() {
        for declaration in fragment {
            map.insert(declaration.key(), declaration.clone());
        }
    }
    map
}

/// Why a style commit was rejected. The tree fills `node` before it returns.
#[derive(Debug, Error, PartialEq)]
pub enum StyleError {
    #[error("unknown node")]
    UnknownNode,
    #[error("{property}: {reason}")]
    Invalid {
        node: Option<u64>,
        property: &'static str,
        reason: &'static str,
    },
    #[error("unknown class token `{token}`")]
    UnknownClass { node: Option<u64>, token: String },
    #[error("unknown property `{name}`")]
    UnknownProperty { node: Option<u64>, name: String },
    #[error("class string is too long")]
    ClassTooLong { node: Option<u64> },
    #[error("{origin} cannot set {key} on this style surface")]
    Forbidden {
        node: Option<u64>,
        origin: &'static str,
        key: &'static str,
    },
}

impl StyleError {
    pub fn with_node(mut self, raw: u64) -> Self {
        match &mut self {
            Self::Invalid { node, .. }
            | Self::UnknownClass { node, .. }
            | Self::UnknownProperty { node, .. }
            | Self::ClassTooLong { node }
            | Self::Forbidden { node, .. } => *node = Some(raw),
            Self::UnknownNode => {}
        }
        self
    }
}

/// Quote a generated TypeScript module from the property registry.
pub fn emit_typescript() -> String {
    let mut out = String::from(
        "// Generated by `uicore`'s style-types binary. Do not edit.\n\
         declare global {\n\
           interface ImpastoColorTokens {}\n\
           interface ImpastoLengthTokens {}\n\
         }\n\
         export type ColorTokenName = keyof ImpastoColorTokens extends never ? string : Extract<keyof ImpastoColorTokens, string>;\n\
         export type LengthTokenName = keyof ImpastoLengthTokens extends never ? string : Extract<keyof ImpastoLengthTokens, string>;\n\
         export type AbsoluteLength = number | { rem: number } | { vw: number } | { vh: number } | { token: LengthTokenName };\n\
         export type Length = AbsoluteLength | `${number}%` | { steps: number };\n\
         export type SizeValue = Length | \"auto\" | \"min-content\" | \"max-content\" | \"fit-content\" | \"stretch\" | { fitContent: Length };\n\
         export type MinSize = Length | \"auto\";\n\
         export type MaxSize = Length | \"none\";\n\
         export type MarginValue = Length | \"auto\";\n\
         export type InsetValue = Length | \"auto\";\n\
         export type BasisValue = SizeValue | \"content\";\n\
         export type FlexValue = number | \"none\" | \"auto\" | \"initial\";\n\
         export type Color = string | readonly [number, number, number, number] | { token: ColorTokenName; opacity?: number };\n\
         export type EffectColor = string | readonly [number, number, number, number];\n\
         export type ShadowStyle = { color: EffectColor; elevation: number };\n\
         export type BoxShadowStyle = { offset: readonly [number, number]; blurSigma: number; spread?: number; color: EffectColor };\n\
         export type GradientStyle = { stops: readonly { offset: number; color: EffectColor }[] } & ({ kind: \"linear\"; from: readonly [number, number]; to: readonly [number, number] } | { kind: \"radial\"; center?: readonly [number, number]; radius: number } | { kind: \"sweep\"; center?: readonly [number, number]; startAngle: number; endAngle: number });\n\
         export type Inherited<T> = T | { inherit: true };\n\
         export type GridLine = \"auto\" | { line: number } | { span: number };\n\
         export type TrackBound = \"auto\" | \"min-content\" | \"max-content\" | { points: number } | { fr: number } | { percent: number };\n\
         export type GridTrack = TrackBound | { min: TrackBound; max: TrackBound };\n\n",
    );
    for group in ["box", "paint", "text"] {
        let title = match group {
            "box" => "BoxStyle",
            "paint" => "PaintStyle",
            _ => "TextStyle",
        };
        out.push_str("export interface ");
        out.push_str(title);
        out.push_str(" {\n");
        for field in field_metas().iter().filter(|field| field.group == group) {
            out.push_str("  ");
            out.push_str(field.name);
            out.push_str("?: ");
            out.push_str(field.ts_type);
            out.push_str(" | null;\n");
        }
        out.push_str("}\n\n");
    }
    out.push_str("export interface ViewStyle extends BoxStyle, PaintStyle, TextStyle {}\n");
    out.push_str(
        "\nexport type StyleInput = ViewStyle | readonly (ViewStyle | false | null | undefined)[] | null | undefined;\n",
    );
    out
}

/// Supported utility families. This is the native contract, not a Tailwind stylesheet.
pub fn emit_utilities() -> String {
    let mut out = String::from(
        "# 原生工具类\n\n\
         由 `style-types` 从属性注册表生成。类名使用 Tailwind 的常用词，默认值、单位和冲突顺序是本框架自己的。\n\n\
         数字间距 `p-4` 是 4 个间距步长，默认一步 4 点。`p-[4px]` 是 4 个场景点。`rem` 留到环境解析，不乘死 16。\n\n\
         同一 class 来源里，后写的同一长属性胜出。`p-4 pl-2` 只覆盖 left。变体优先级是 base、响应式、dark、hover、focus、active、disabled。\n\n\
         `class=\"flex\"` 只写 display，不改方向。横排用 `flex-row`。\n\n\
         ## 属性\n\n",
    );
    for field in field_metas() {
        out.push_str("- `");
        out.push_str(field.name);
        out.push_str("` (`");
        out.push_str(field.group);
        out.push_str("`): `");
        out.push_str(field.ts_type);
        out.push_str("`\n");
    }
    out.push_str(
        "\n## 类族\n\n\
         - display: `flex` `grid` `block` `flow-root` `hidden`\n\
         - 尺寸: `w-` `h-` `size-` `min-w-` `max-w-` `min-h-` `max-h-`，以及 `min` `max` `fit` `screen` `full` `auto`\n\
         - Flex: `flex-` `grow` `shrink` `basis-` `flex-row` `flex-col` `flex-wrap`\n\
         - 对齐: `items-` `self-` `justify-` `content-`\n\
         - 间距: `p-` `px-` `py-` `m-` `mx-` `my-` `gap-`，含负值和 `auto`\n\
         - 定位: `relative` `absolute` `inset-` `top-` `right-` `bottom-` `left-`\n\
         - Grid: `grid-cols-` `grid-rows-` `col-span-` `row-span-`\n\
         - 颜色: `bg-` `text-` `border-` `outline-`，主题 token、hex 和 `/透明度`\n\
         - 文字: `text-` `font-` `leading-` `text-left` `truncate` `underline` `line-through`\n\
         - 装饰: `rounded` `border` `outline` `shadow` `blur` `backdrop`\n\
         - 变换: `opacity-` `translate-` `rotate-` `scale-` `origin-` `z-`\n\
         - 图片: `object-fill` `object-contain` `object-cover`\n\
         - 变体: `sm` `md` `lg` `xl` `2xl` `dark` `hover` `focus` `active` `disabled`\n\n\
         不支持 `calc`、`var`、任意属性、`group`/`peer`、`container`、`important`、`fixed`、`sticky`、`subgrid`、`float` 和 `order`。未知 token 报错，不会当成 `auto`。\n",
    );
    out
}
