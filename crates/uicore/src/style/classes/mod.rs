//! Utility-class shorthand. The parser emits the same declarations as `style`.
//!
//! Variants compare by `(state, dark, breakpoint, condition count)`, then source order.
//! `display: flex` does not write a direction.

use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;

use super::environment::{CLASS_BYTE_LIMIT, DepMask, Environment, Interaction};
use super::property::{display_flex, expand_flex, expand_gap, expand_inset, expand_padding};
use super::values::{
    AbsoluteLength, BasisValue, ContentAlign, DisplayMode, FlexDir, FlexShorthand, FlexWrapMode,
    GridPlacement, GridTrack, InsetValue, ItemAlign, Length, LimitValue, MarginValue, OverflowMode,
    PaintColor, PositionMode, SizeValue, TrackList,
};
use super::{Declaration, Property, Style, StyleError};

const SYNTAX_VERSION: u32 = 1;

#[derive(Clone, Debug)]
struct ClassRule {
    order: u32,
    mask: u32,
    decls: Vec<Declaration>,
}

/// Parsed rules for one class string. Nodes hold this in an `Arc`.
#[derive(Clone, Debug)]
pub struct ParsedClass {
    rules: Vec<ClassRule>,
    pub deps: DepMask,
    bytes: usize,
}

impl ParsedClass {
    pub fn select(&self, env: &Environment, state: Interaction) -> Result<Style, StyleError> {
        let mut matched: Vec<&ClassRule> = self
            .rules
            .iter()
            .filter(|rule| variant_matches(rule.mask, env, state))
            .collect();
        matched.sort_by_key(|rule| (rank_key(rule.mask), rule.order));
        let mut style = Style::new();
        for rule in matched {
            style.push_fragment(resolve_tokens(&rule.decls, env)?)?;
        }
        Ok(style)
    }
}

fn variant_matches(mask: u32, env: &Environment, state: Interaction) -> bool {
    if mask & SM != 0 && !env.breakpoint_matches(0) {
        return false;
    }
    if mask & MD != 0 && !env.breakpoint_matches(1) {
        return false;
    }
    if mask & LG != 0 && !env.breakpoint_matches(2) {
        return false;
    }
    if mask & XL != 0 && !env.breakpoint_matches(3) {
        return false;
    }
    if mask & XL2 != 0 && !env.breakpoint_matches(4) {
        return false;
    }
    if mask & DARK != 0 && !env.dark {
        return false;
    }
    if mask & HOVER != 0 && !state.hover {
        return false;
    }
    if mask & FOCUS != 0 && !state.focus {
        return false;
    }
    if mask & ACTIVE != 0 && !state.active {
        return false;
    }
    if mask & DISABLED != 0 && !state.disabled {
        return false;
    }
    true
}

const SM: u32 = 1 << 0;
const MD: u32 = 1 << 1;
const LG: u32 = 1 << 2;
const XL: u32 = 1 << 3;
const XL2: u32 = 1 << 4;
const DARK: u32 = 1 << 5;
const HOVER: u32 = 1 << 6;
const FOCUS: u32 = 1 << 7;
const ACTIVE: u32 = 1 << 8;
const DISABLED: u32 = 1 << 9;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    version: u32,
    text: Arc<str>,
}

struct CacheEntry {
    parsed: Arc<ParsedClass>,
    bytes: usize,
}

/// Session cache. A hit promotes the entry. Eviction drops the cache reference
/// only; live nodes keep their `Arc`. The byte budget counts the class text
/// and the stored rules, not the whole tree.
#[derive(Debug)]
pub struct ClassCache {
    map: LruCache<CacheKey, CacheEntry>,
    bytes: usize,
    max_bytes: usize,
}

impl Default for ClassCache {
    fn default() -> Self {
        Self::new(256, 256 * 1024)
    }
}

impl ClassCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        let cap = NonZeroUsize::new(max_entries.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            map: LruCache::new(cap),
            bytes: 0,
            max_bytes: max_bytes.max(1),
        }
    }

    pub fn contains(&self, class_text: &str) -> bool {
        let key = CacheKey {
            version: SYNTAX_VERSION,
            text: Arc::from(class_text),
        };
        self.map.contains(&key)
    }

    pub fn get(&mut self, class_text: &str) -> Result<Arc<ParsedClass>, StyleError> {
        if class_text.len() > CLASS_BYTE_LIMIT {
            return Err(StyleError::ClassTooLong { node: None });
        }
        let key = CacheKey {
            version: SYNTAX_VERSION,
            text: Arc::from(class_text),
        };
        if let Some(found) = self.map.get(&key) {
            return Ok(Arc::clone(&found.parsed));
        }
        let parsed = Arc::new(parse_class(class_text)?);
        let bytes = class_text.len().saturating_add(parsed.bytes);
        self.bytes = self.bytes.saturating_add(bytes);
        if let Some((_, removed)) = self.map.push(
            key,
            CacheEntry {
                parsed: Arc::clone(&parsed),
                bytes,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(removed.bytes);
        }
        self.evict();
        Ok(parsed)
    }

    fn evict(&mut self) {
        while self.map.len() > self.map.cap().get() || self.bytes > self.max_bytes {
            let Some((_, removed)) = self.map.pop_lru() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.bytes);
        }
    }
}

pub fn parse_class(input: &str) -> Result<ParsedClass, StyleError> {
    if input.len() > CLASS_BYTE_LIMIT {
        return Err(StyleError::ClassTooLong { node: None });
    }
    let mut rules = Vec::new();
    let mut deps = DepMask::empty();
    for (order, token) in class_tokens(input)?.into_iter().enumerate() {
        let (mask, utility) = split_variants(token)?;
        deps |= deps_for(mask);
        let decls = parse_utility(utility)?;
        for declaration in &decls {
            if let Declaration::Set(property) = declaration {
                deps |= super::dependencies(property);
            }
        }
        let order = u32::try_from(order).unwrap_or(u32::MAX);
        rules.push(ClassRule { order, mask, decls });
    }
    let bytes = input.len().saturating_add(estimate_rules(&rules));
    Ok(ParsedClass { rules, deps, bytes })
}

/// Split a class string. Whitespace inside brackets, parentheses, or quotes stays in the token.
pub(super) fn class_tokens(input: &str) -> Result<Vec<&str>, StyleError> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut round = 0i32;
    let mut square = 0i32;
    let mut quote = None::<char>;
    for (index, ch) in input.char_indices() {
        if let Some(mark) = quote {
            if ch == mark {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                start.get_or_insert(index);
                quote = Some(ch);
            }
            '(' => {
                start.get_or_insert(index);
                round += 1;
            }
            ')' => {
                start.get_or_insert(index);
                round -= 1;
                if round < 0 {
                    return Err(StyleError::UnknownClass {
                        node: None,
                        token: input[index..].to_owned(),
                    });
                }
            }
            '[' => {
                start.get_or_insert(index);
                square += 1;
            }
            ']' => {
                start.get_or_insert(index);
                square -= 1;
                if square < 0 {
                    return Err(StyleError::UnknownClass {
                        node: None,
                        token: input[index..].to_owned(),
                    });
                }
            }
            ch if ch.is_whitespace() && round == 0 && square == 0 => {
                if let Some(from) = start.take() {
                    tokens.push(&input[from..index]);
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if quote.is_some() || round != 0 || square != 0 {
        return Err(StyleError::UnknownClass {
            node: None,
            token: input.to_owned(),
        });
    }
    if let Some(from) = start {
        tokens.push(&input[from..]);
    }
    Ok(tokens)
}

fn estimate_rules(rules: &[ClassRule]) -> usize {
    rules
        .iter()
        .map(|rule| rule.decls.len().saturating_mul(24))
        .sum()
}

/// Explicit order: state, then dark, then breakpoint, then how many conditions matched.
fn rank_key(mask: u32) -> (u32, u32, u32, u32) {
    let state = if mask & DISABLED != 0 {
        6
    } else if mask & ACTIVE != 0 {
        5
    } else if mask & FOCUS != 0 {
        4
    } else if mask & HOVER != 0 {
        3
    } else {
        0
    };
    let dark = u32::from(mask & DARK != 0);
    let breakpoint = if mask & XL2 != 0 {
        5
    } else if mask & XL != 0 {
        4
    } else if mask & LG != 0 {
        3
    } else if mask & MD != 0 {
        2
    } else if mask & SM != 0 {
        1
    } else {
        0
    };
    (state, dark, breakpoint, mask.count_ones())
}

fn deps_for(mask: u32) -> DepMask {
    let mut deps = DepMask::empty();
    if mask & (SM | MD | LG | XL | XL2) != 0 {
        deps |= DepMask::WINDOW;
    }
    if mask & DARK != 0 {
        deps |= DepMask::DARK;
    }
    if mask & HOVER != 0 {
        deps |= DepMask::HOVER;
    }
    if mask & FOCUS != 0 {
        deps |= DepMask::FOCUS;
    }
    if mask & ACTIVE != 0 {
        deps |= DepMask::ACTIVE;
    }
    if mask & DISABLED != 0 {
        deps |= DepMask::DISABLED;
    }
    deps
}

fn split_variants(token: &str) -> Result<(u32, &str), StyleError> {
    let mut mask = 0u32;
    let mut rest = token;
    while let Some((head, tail)) = rest.split_once(':') {
        let bit = variant_bit(head).ok_or_else(|| unknown(token))?;
        let breakpoints = SM | MD | LG | XL | XL2;
        if bit & breakpoints != 0 && mask & breakpoints != 0 {
            return Err(unknown(token));
        }
        if mask & bit != 0 {
            return Err(unknown(token));
        }
        mask |= bit;
        rest = tail;
    }
    if rest.is_empty() {
        return Err(unknown(token));
    }
    Ok((mask, rest))
}

fn variant_bit(name: &str) -> Option<u32> {
    Some(match name {
        "sm" => SM,
        "md" => MD,
        "lg" => LG,
        "xl" => XL,
        "2xl" => XL2,
        "dark" => DARK,
        "hover" => HOVER,
        "focus" => FOCUS,
        "active" => ACTIVE,
        "disabled" => DISABLED,
        _ => return None,
    })
}

fn unknown(token: &str) -> StyleError {
    StyleError::UnknownClass {
        node: None,
        token: token.to_owned(),
    }
}

fn resolve_tokens(
    decls: &[Declaration],
    env: &Environment,
) -> Result<Vec<Declaration>, StyleError> {
    let mut out = Vec::with_capacity(decls.len());
    for declaration in decls {
        if let Declaration::Set(Property::FontFamily(name)) = declaration
            && let Some(role_name) = name.strip_prefix("role:")
        {
            let role = env.role(role_name).ok_or(StyleError::Invalid {
                node: None,
                property: "fontSize",
                reason: "unknown type role",
            })?;
            out.push(Declaration::set(Property::FontSize(
                AbsoluteLength::points(role.size),
            )));
            out.push(Declaration::set(Property::FontWeight(role.weight)));
            out.push(Declaration::set(Property::LineHeight(
                super::values::LineHeightValue::factor(role.line_height),
            )));
            continue;
        }
        out.push(declaration.clone());
    }
    Ok(out)
}

/// A type role stays a token until [`ParsedClass::select`] reads the environment.
fn expand_role(name: &str) -> Vec<Declaration> {
    vec![Declaration::set(Property::FontFamily(format!(
        "role:{name}"
    )))]
}

pub fn parse_utility(token: &str) -> Result<Vec<Declaration>, StyleError> {
    if let Some(rest) = token.strip_prefix('-') {
        if rest.is_empty() {
            return Err(unknown(token));
        }
        let decls = parse_positive(rest).map_err(|_| unknown(token))?;
        return negate(token, decls);
    }
    parse_positive(token).map_err(|_| unknown(token))
}

fn negate(token: &str, decls: Vec<Declaration>) -> Result<Vec<Declaration>, StyleError> {
    let mut out = Vec::with_capacity(decls.len());
    for declaration in decls {
        let Declaration::Set(property) = declaration else {
            return Err(unknown(token));
        };
        let property = match property {
            Property::MarginTop(MarginValue::Length(length)) => {
                Property::MarginTop(MarginValue::Length(negate_length(length)?))
            }
            Property::MarginRight(MarginValue::Length(length)) => {
                Property::MarginRight(MarginValue::Length(negate_length(length)?))
            }
            Property::MarginBottom(MarginValue::Length(length)) => {
                Property::MarginBottom(MarginValue::Length(negate_length(length)?))
            }
            Property::MarginLeft(MarginValue::Length(length)) => {
                Property::MarginLeft(MarginValue::Length(negate_length(length)?))
            }
            Property::TranslateX(length) => Property::TranslateX(negate_length(length)?),
            Property::TranslateY(length) => Property::TranslateY(negate_length(length)?),
            Property::Top(InsetValue::Length(length)) => {
                Property::Top(InsetValue::Length(negate_length(length)?))
            }
            Property::Right(InsetValue::Length(length)) => {
                Property::Right(InsetValue::Length(negate_length(length)?))
            }
            Property::Bottom(InsetValue::Length(length)) => {
                Property::Bottom(InsetValue::Length(negate_length(length)?))
            }
            Property::Left(InsetValue::Length(length)) => {
                Property::Left(InsetValue::Length(negate_length(length)?))
            }
            _ => return Err(unknown(token)),
        };
        property.validate().map_err(|_| unknown(token))?;
        out.push(Declaration::set(property));
    }
    Ok(out)
}

fn parse_positive(token: &str) -> Result<Vec<Declaration>, ()> {
    for (prefix, make) in [
        ("blur", Property::Blur as fn(f32) -> Property),
        ("backdrop-blur", Property::Backdrop),
        ("backdrop", Property::Backdrop),
    ] {
        if token == prefix {
            return Ok(vec![Declaration::set(make(8.0))]);
        }
        if let Some(arg) = token.strip_prefix(&format!("{prefix}-")) {
            let value = match arg {
                "none" => 0.0,
                "sm" => 4.0,
                "md" => 12.0,
                "lg" => 16.0,
                "xl" => 24.0,
                _ => number_arg(arg)?,
            };
            if value < 0.0 {
                return Err(());
            }
            return Ok(vec![Declaration::set(make(value))]);
        }
    }
    if token == "shadow" || token.starts_with("shadow-") {
        let elevation = match token {
            "shadow-none" => 0.0,
            "shadow-sm" => 2.0,
            "shadow" | "shadow-md" => 4.0,
            "shadow-lg" => 8.0,
            "shadow-xl" => 16.0,
            _ => return Err(()),
        };
        return Ok(vec![Declaration::set(Property::Shadow(protocol::Shadow {
            color: [0, 0, 0, 64],
            elevation,
        }))]);
    }
    if let Some(decls) = exact(token) {
        return Ok(decls);
    }
    let (prefix, arg) = split_prefix(token).ok_or(())?;
    by_prefix(prefix, arg)
}

fn exact(token: &str) -> Option<Vec<Declaration>> {
    let decls = match token {
        "hidden" => vec![Declaration::set(Property::Display(DisplayMode::None))],
        "block" => vec![Declaration::set(Property::Display(DisplayMode::Block))],
        "grid" => vec![Declaration::set(Property::Display(DisplayMode::Grid))],
        "flex" => display_flex(),
        "flex-row" => vec![Declaration::set(Property::FlexDirection(FlexDir::Row))],
        "flex-col" => vec![Declaration::set(Property::FlexDirection(FlexDir::Column))],
        "flex-row-reverse" => {
            vec![Declaration::set(Property::FlexDirection(
                FlexDir::RowReverse,
            ))]
        }
        "flex-col-reverse" => {
            vec![Declaration::set(Property::FlexDirection(
                FlexDir::ColumnReverse,
            ))]
        }
        "flex-wrap" => vec![Declaration::set(Property::FlexWrap(FlexWrapMode::Wrap))],
        "flex-nowrap" => vec![Declaration::set(Property::FlexWrap(FlexWrapMode::NoWrap))],
        "flex-none" => expand_flex(FlexShorthand::None).ok()?,
        "flex-auto" => expand_flex(FlexShorthand::Auto).ok()?,
        "flex-initial" => expand_flex(FlexShorthand::Initial).ok()?,
        "grow" => vec![Declaration::set(Property::FlexGrow(1.0))],
        "shrink" => vec![Declaration::set(Property::FlexShrink(1.0))],
        "relative" => vec![Declaration::set(Property::Position(PositionMode::Relative))],
        "absolute" => vec![Declaration::set(Property::Position(PositionMode::Absolute))],
        "overflow-hidden" => vec![Declaration::set(Property::Overflow(OverflowMode::Hidden))],
        "overflow-visible" => vec![Declaration::set(Property::Overflow(OverflowMode::Visible))],
        "object-fill" => vec![Declaration::set(Property::ObjectFit(
            protocol::ContentMode::Fill,
        ))],
        "object-contain" => vec![Declaration::set(Property::ObjectFit(
            protocol::ContentMode::Fit,
        ))],
        "object-cover" => vec![Declaration::set(Property::ObjectFit(
            protocol::ContentMode::Crop,
        ))],
        "rounded" => vec![Declaration::set(Property::BorderRadius(Length::token(
            "radius.md",
        )))],
        "rounded-none" => vec![Declaration::set(Property::BorderRadius(Length::points(
            0.0,
        )))],
        "rounded-full" => vec![Declaration::set(Property::BorderRadius(Length::token(
            "radius.full",
        )))],
        "border" => vec![Declaration::set(Property::BorderWidth(Length::points(1.0)))],
        "items-start" => align_items(ItemAlign::FlexStart),
        "items-center" => align_items(ItemAlign::Center),
        "items-end" => align_items(ItemAlign::End),
        "items-stretch" => align_items(ItemAlign::Stretch),
        "items-baseline" => align_items(ItemAlign::Baseline),
        "self-auto" => vec![Declaration::set(Property::AlignSelf(ItemAlign::Auto))],
        "self-start" => vec![Declaration::set(Property::AlignSelf(ItemAlign::FlexStart))],
        "self-center" => vec![Declaration::set(Property::AlignSelf(ItemAlign::Center))],
        "self-end" => vec![Declaration::set(Property::AlignSelf(ItemAlign::End))],
        "self-stretch" => vec![Declaration::set(Property::AlignSelf(ItemAlign::Stretch))],
        "self-baseline" => vec![Declaration::set(Property::AlignSelf(ItemAlign::Baseline))],
        "justify-start" => justify(ContentAlign::FlexStart),
        "justify-center" => justify(ContentAlign::Center),
        "justify-end" => justify(ContentAlign::End),
        "justify-between" => justify(ContentAlign::SpaceBetween),
        "justify-around" => justify(ContentAlign::SpaceAround),
        "justify-evenly" => justify(ContentAlign::SpaceEvenly),
        "text-left" => vec![Declaration::set(Property::TextAlign(
            protocol::TextAlign::Left,
        ))],
        "text-center" => vec![Declaration::set(Property::TextAlign(
            protocol::TextAlign::Center,
        ))],
        "text-right" => vec![Declaration::set(Property::TextAlign(
            protocol::TextAlign::Right,
        ))],
        "font-normal" => vec![Declaration::set(Property::FontWeight(400))],
        "font-medium" => vec![Declaration::set(Property::FontWeight(500))],
        "font-semibold" => vec![Declaration::set(Property::FontWeight(600))],
        "font-bold" => vec![Declaration::set(Property::FontWeight(700))],
        _ => return None,
    };
    Some(decls)
}

fn align_items(align: ItemAlign) -> Vec<Declaration> {
    vec![Declaration::set(Property::AlignItems(align))]
}

fn justify(align: ContentAlign) -> Vec<Declaration> {
    vec![Declaration::set(Property::JustifyContent(align))]
}

fn split_prefix(token: &str) -> Option<(&str, &str)> {
    const PREFIXES: &[&str] = &[
        "grid-cols",
        "grid-rows",
        "translate-x",
        "translate-y",
        "col-span",
        "col-start",
        "col-end",
        "row-span",
        "row-start",
        "row-end",
        "scale-x",
        "scale-y",
        "min-w",
        "max-w",
        "min-h",
        "max-h",
        "gap-x",
        "gap-y",
        "basis",
        "rounded",
        "opacity",
        "border",
        "outline",
        "rotate",
        "scale",
        "inset",
        "right",
        "bottom",
        "text",
        "flex",
        "grow",
        "shrink",
        "gap",
        "size",
        "px",
        "py",
        "pt",
        "pr",
        "pb",
        "pl",
        "mx",
        "my",
        "mt",
        "mr",
        "mb",
        "ml",
        "bg",
        "top",
        "left",
        "w",
        "h",
        "p",
        "m",
    ];
    for prefix in PREFIXES {
        if let Some(rest) = token.strip_prefix(prefix)
            && let Some(arg) = rest.strip_prefix('-')
            && !arg.is_empty()
        {
            return Some((*prefix, arg));
        }
    }
    None
}

fn by_prefix(prefix: &str, arg: &str) -> Result<Vec<Declaration>, ()> {
    match prefix {
        "p" => expand_padding(spacing(arg)?).map_err(|_| ()),
        "px" => pad_x(spacing(arg)?),
        "py" => pad_y(spacing(arg)?),
        "pt" => Ok(vec![Declaration::set(Property::PaddingTop(spacing(arg)?))]),
        "pr" => Ok(vec![Declaration::set(Property::PaddingRight(spacing(
            arg,
        )?))]),
        "pb" => Ok(vec![Declaration::set(Property::PaddingBottom(spacing(
            arg,
        )?))]),
        "pl" => Ok(vec![Declaration::set(Property::PaddingLeft(spacing(arg)?))]),
        "m" => margin_all(spacing(arg)?),
        "mx" => margin_x(spacing(arg)?),
        "my" => margin_y(spacing(arg)?),
        "mt" => Ok(margin_edge(Property::MarginTop, spacing(arg)?)),
        "mr" => Ok(margin_edge(Property::MarginRight, spacing(arg)?)),
        "mb" => Ok(margin_edge(Property::MarginBottom, spacing(arg)?)),
        "ml" => Ok(margin_edge(Property::MarginLeft, spacing(arg)?)),
        "gap" => expand_gap(spacing(arg)?).map_err(|_| ()),
        "gap-x" => Ok(vec![Declaration::set(Property::ColumnGap(spacing(arg)?))]),
        "gap-y" => Ok(vec![Declaration::set(Property::RowGap(spacing(arg)?))]),
        "w" => Ok(vec![Declaration::set(Property::Width(axis_arg(
            arg, true,
        )?))]),
        "h" => Ok(vec![Declaration::set(Property::Height(axis_arg(
            arg, false,
        )?))]),
        "size" => {
            let width = axis_arg(arg, true)?;
            let height = axis_arg(arg, false)?;
            Ok(vec![
                Declaration::set(Property::Width(width)),
                Declaration::set(Property::Height(height)),
            ])
        }
        "min-w" => Ok(vec![Declaration::set(Property::MinWidth(min_axis(
            arg, true,
        )?))]),
        "min-h" => Ok(vec![Declaration::set(Property::MinHeight(min_axis(
            arg, false,
        )?))]),
        "max-w" => Ok(vec![Declaration::set(Property::MaxWidth(max_axis(
            arg, true,
        )?))]),
        "max-h" => Ok(vec![Declaration::set(Property::MaxHeight(max_axis(
            arg, false,
        )?))]),
        "basis" => Ok(vec![Declaration::set(Property::FlexBasis(
            BasisValue::Size(axis_arg(arg, true)?),
        ))]),
        "grow" => Ok(vec![Declaration::set(Property::FlexGrow(number_arg(arg)?))]),
        "shrink" => Ok(vec![Declaration::set(Property::FlexShrink(number_arg(
            arg,
        )?))]),
        "flex" => flex_arg(arg),
        "text" => text_arg(arg),
        "bg" => Ok(vec![Declaration::set(Property::BackgroundColor(
            color_arg(arg)?,
        ))]),
        "border" => border_arg(arg),
        "outline" => outline_arg(arg),
        "rounded" => Ok(vec![Declaration::set(Property::BorderRadius(radius_arg(
            arg,
        )?))]),
        "opacity" => Ok(vec![Declaration::set(Property::Opacity(opacity_arg(arg)?))]),
        "rotate" => Ok(vec![Declaration::set(Property::Rotate(degrees(arg)?))]),
        "scale" => {
            let value = scale_arg(arg)?;
            Ok(vec![
                Declaration::set(Property::ScaleX(value)),
                Declaration::set(Property::ScaleY(value)),
            ])
        }
        "scale-x" => Ok(vec![Declaration::set(Property::ScaleX(scale_arg(arg)?))]),
        "scale-y" => Ok(vec![Declaration::set(Property::ScaleY(scale_arg(arg)?))]),
        "translate-x" => Ok(vec![Declaration::set(Property::TranslateX(spacing(arg)?))]),
        "translate-y" => Ok(vec![Declaration::set(Property::TranslateY(spacing(arg)?))]),
        "inset" => expand_inset(InsetValue::Length(spacing(arg)?)).map_err(|_| ()),
        "top" => Ok(vec![Declaration::set(Property::Top(inset_arg(arg)?))]),
        "right" => Ok(vec![Declaration::set(Property::Right(inset_arg(arg)?))]),
        "bottom" => Ok(vec![Declaration::set(Property::Bottom(inset_arg(arg)?))]),
        "left" => Ok(vec![Declaration::set(Property::Left(inset_arg(arg)?))]),
        "grid-cols" => grid_tracks(true, arg),
        "grid-rows" => grid_tracks(false, arg),
        "col-span" => Ok(vec![Declaration::set(Property::GridColumnStart(span_arg(
            arg,
        )?))]),
        "row-span" => Ok(vec![Declaration::set(Property::GridRowStart(span_arg(
            arg,
        )?))]),
        "col-start" => Ok(vec![Declaration::set(Property::GridColumnStart(line_arg(
            arg,
        )?))]),
        "col-end" => Ok(vec![Declaration::set(Property::GridColumnEnd(line_arg(
            arg,
        )?))]),
        "row-start" => Ok(vec![Declaration::set(Property::GridRowStart(line_arg(
            arg,
        )?))]),
        "row-end" => Ok(vec![Declaration::set(Property::GridRowEnd(line_arg(arg)?))]),
        _ => Err(()),
    }
}

fn negate_length(length: Length) -> Result<Length, StyleError> {
    Ok(match length {
        Length::Points(value) => Length::points(-value),
        Length::Fraction(value) => Length::fraction(-value),
        Length::Rem(value) => Length::rem(-value),
        Length::Steps(value) => Length::steps(-value),
        Length::ViewportWidth(value) => Length::ViewportWidth(super::values::canonical(-value)),
        Length::ViewportHeight(value) => Length::ViewportHeight(super::values::canonical(-value)),
        Length::Token(_) => return Err(unknown("token")),
    })
}

fn pad_x(value: Length) -> Result<Vec<Declaration>, ()> {
    Ok(vec![
        Declaration::set(Property::PaddingLeft(value.clone())),
        Declaration::set(Property::PaddingRight(value)),
    ])
}

fn pad_y(value: Length) -> Result<Vec<Declaration>, ()> {
    Ok(vec![
        Declaration::set(Property::PaddingTop(value.clone())),
        Declaration::set(Property::PaddingBottom(value)),
    ])
}

fn margin_all(value: Length) -> Result<Vec<Declaration>, ()> {
    let margin = MarginValue::Length(value);
    Ok(vec![
        Declaration::set(Property::MarginTop(margin.clone())),
        Declaration::set(Property::MarginRight(margin.clone())),
        Declaration::set(Property::MarginBottom(margin.clone())),
        Declaration::set(Property::MarginLeft(margin)),
    ])
}

fn margin_x(value: Length) -> Result<Vec<Declaration>, ()> {
    let margin = MarginValue::Length(value);
    Ok(vec![
        Declaration::set(Property::MarginLeft(margin.clone())),
        Declaration::set(Property::MarginRight(margin)),
    ])
}

fn margin_y(value: Length) -> Result<Vec<Declaration>, ()> {
    let margin = MarginValue::Length(value);
    Ok(vec![
        Declaration::set(Property::MarginTop(margin.clone())),
        Declaration::set(Property::MarginBottom(margin)),
    ])
}

fn margin_edge(make: fn(MarginValue) -> Property, value: Length) -> Vec<Declaration> {
    vec![Declaration::set(make(MarginValue::Length(value)))]
}

fn flex_arg(arg: &str) -> Result<Vec<Declaration>, ()> {
    let flex = match arg {
        "none" => FlexShorthand::None,
        "auto" => FlexShorthand::Auto,
        "initial" => FlexShorthand::Initial,
        _ => FlexShorthand::Number(number_arg(arg)?),
    };
    expand_flex(flex).map_err(|_| ())
}

fn text_arg(arg: &str) -> Result<Vec<Declaration>, ()> {
    if let Some(size) = font_size_name(arg) {
        return Ok(vec![Declaration::set(Property::FontSize(size))]);
    }
    if matches!(
        arg,
        "display" | "headline" | "title" | "body" | "label" | "caption"
    ) {
        return Ok(expand_role(arg));
    }
    Ok(vec![Declaration::set(Property::Color(color_arg(arg)?))])
}

fn font_size_name(arg: &str) -> Option<AbsoluteLength> {
    let name = match arg {
        "xs" => "text.xs",
        "sm" => "text.sm",
        "base" => "text.base",
        "lg" => "text.lg",
        "xl" => "text.xl",
        "2xl" => "text.2xl",
        "3xl" => "text.3xl",
        "4xl" => "text.4xl",
        _ => return None,
    };
    Some(AbsoluteLength::token(name))
}

fn border_arg(arg: &str) -> Result<Vec<Declaration>, ()> {
    if let Ok(width) = spacing(arg) {
        return Ok(vec![Declaration::set(Property::BorderWidth(width))]);
    }
    Ok(vec![Declaration::set(Property::BorderColor(color_arg(
        arg,
    )?))])
}

fn outline_arg(arg: &str) -> Result<Vec<Declaration>, ()> {
    if let Ok(width) = spacing(arg) {
        return Ok(vec![Declaration::set(Property::OutlineWidth(width))]);
    }
    Ok(vec![Declaration::set(Property::OutlineColor(color_arg(
        arg,
    )?))])
}

fn radius_arg(arg: &str) -> Result<Length, ()> {
    Ok(match arg {
        "none" => Length::points(0.0),
        "sm" => Length::token("radius.sm"),
        "md" => Length::token("radius.md"),
        "lg" => Length::token("radius.lg"),
        "xl" => Length::token("radius.xl"),
        "2xl" => Length::token("radius.2xl"),
        "full" => Length::token("radius.full"),
        _ => length_value(arg)?,
    })
}

fn opacity_arg(arg: &str) -> Result<f32, ()> {
    if let Some(inner) = bracket(arg) {
        let value = parse_f32(inner)?;
        if (0.0..=1.0).contains(&value) {
            return Ok(value);
        }
        return Err(());
    }
    let step = parse_f32(arg)?;
    if !(0.0..=100.0).contains(&step) {
        return Err(());
    }
    Ok(step / 100.0)
}

fn scale_arg(arg: &str) -> Result<f32, ()> {
    if let Some(inner) = bracket(arg) {
        return parse_f32(inner);
    }
    let step = parse_f32(arg)?;
    Ok(step / 100.0)
}

fn degrees(arg: &str) -> Result<f32, ()> {
    let value = if let Some(inner) = bracket(arg) {
        parse_f32(inner)?
    } else {
        parse_f32(arg)?
    };
    Ok(value * std::f32::consts::PI / 180.0)
}

fn spacing(arg: &str) -> Result<Length, ()> {
    if arg == "px" {
        return Ok(Length::points(1.0));
    }
    if let Some(inner) = bracket(arg) {
        return arbitrary_length(inner);
    }
    Ok(Length::steps(parse_f32(arg)?))
}

fn axis_arg(arg: &str, horizontal: bool) -> Result<SizeValue, ()> {
    Ok(match arg {
        "auto" => SizeValue::Auto,
        "full" => SizeValue::fraction(1.0),
        "screen" => SizeValue::Length(if horizontal {
            Length::ViewportWidth(1.0)
        } else {
            Length::ViewportHeight(1.0)
        }),
        "min" => SizeValue::MinContent,
        "max" => SizeValue::MaxContent,
        "fit" => SizeValue::FitContent,
        "0" => SizeValue::points(0.0),
        _ => {
            if let Some((num, den)) = arg.split_once('/') {
                let num = parse_f32(num)?;
                let den = parse_f32(den)?;
                if den == 0.0 {
                    return Err(());
                }
                SizeValue::fraction(num / den)
            } else if arg == "px" {
                SizeValue::points(1.0)
            } else if let Some(inner) = bracket(arg) {
                SizeValue::Length(arbitrary_length(inner)?)
            } else {
                SizeValue::Length(spacing(arg)?)
            }
        }
    })
}

fn arbitrary_length(inner: &str) -> Result<Length, ()> {
    if let Some(number) = inner.strip_suffix("px") {
        return Ok(Length::points(parse_f32(number)?));
    }
    if let Some(number) = inner.strip_suffix('%') {
        return Ok(Length::fraction(parse_f32(number)? / 100.0));
    }
    if let Some(number) = inner.strip_suffix("rem") {
        return Ok(Length::rem(parse_f32(number)?));
    }
    if let Some(number) = inner.strip_suffix("vw") {
        return Ok(Length::ViewportWidth(super::values::canonical(
            parse_f32(number)? / 100.0,
        )));
    }
    if let Some(number) = inner.strip_suffix("vh") {
        return Ok(Length::ViewportHeight(super::values::canonical(
            parse_f32(number)? / 100.0,
        )));
    }
    let number = parse_f32(inner)?;
    if number == 0.0 {
        return Ok(Length::points(0.0));
    }
    Err(())
}

fn limit_from_size(size: SizeValue) -> Result<LimitValue, ()> {
    match size {
        SizeValue::Auto => Ok(LimitValue::Auto),
        SizeValue::Length(length) => Ok(LimitValue::Length(length)),
        SizeValue::MinContent
        | SizeValue::MaxContent
        | SizeValue::FitContent
        | SizeValue::FitContentLimit(_)
        | SizeValue::Stretch => Err(()),
    }
}

fn min_axis(arg: &str, horizontal: bool) -> Result<LimitValue, ()> {
    if arg == "auto" {
        Ok(LimitValue::Auto)
    } else {
        limit_from_size(axis_arg(arg, horizontal)?)
    }
}

fn max_axis(arg: &str, horizontal: bool) -> Result<LimitValue, ()> {
    if arg == "none" {
        Ok(LimitValue::Auto)
    } else {
        limit_from_size(axis_arg(arg, horizontal)?)
    }
}

fn inset_arg(arg: &str) -> Result<InsetValue, ()> {
    if arg == "auto" {
        Ok(InsetValue::auto())
    } else {
        Ok(InsetValue::Length(spacing(arg)?))
    }
}

fn length_value(arg: &str) -> Result<Length, ()> {
    if let Some(inner) = bracket(arg) {
        return arbitrary_length(inner);
    }
    spacing(arg)
}

fn number_arg(arg: &str) -> Result<f32, ()> {
    if let Some(inner) = bracket(arg) {
        return parse_f32(inner);
    }
    parse_f32(arg)
}

fn grid_tracks(columns: bool, arg: &str) -> Result<Vec<Declaration>, ()> {
    let count = whole_number(parse_f32(arg)?, 1, 24)?;
    let tracks = TrackList::tracks(vec![GridTrack::fill(); usize::from(count)]);
    let property = if columns {
        Property::GridTemplateColumns(tracks)
    } else {
        Property::GridTemplateRows(tracks)
    };
    Ok(vec![Declaration::set(property)])
}

fn span_arg(arg: &str) -> Result<GridPlacement, ()> {
    let count = whole_number(parse_f32(arg)?, 1, 64)?;
    Ok(GridPlacement::Span(u16::from(count)))
}

fn line_arg(arg: &str) -> Result<GridPlacement, ()> {
    let line = signed_whole(parse_f32(arg)?, 64)?;
    if line == 0 {
        return Err(());
    }
    Ok(GridPlacement::Line(line))
}

fn whole_number(value: f32, min: u8, max: u8) -> Result<u8, ()> {
    if value < f32::from(min) || value > f32::from(max) {
        return Err(());
    }
    let signed = signed_whole(value, i16::from(max))?;
    u8::try_from(signed).map_err(|_| ())
}

fn signed_whole(value: f32, max: i16) -> Result<i16, ()> {
    if value < -f32::from(max) || value > f32::from(max) {
        return Err(());
    }
    let negative = value < 0.0;
    let mut left = value.abs();
    let mut out = 0i16;
    while left >= 1.0 {
        out = out.checked_add(1).ok_or(())?;
        left -= 1.0;
    }
    if left > 0.001 {
        return Err(());
    }
    if negative { Ok(-out) } else { Ok(out) }
}

fn color_arg(arg: &str) -> Result<PaintColor, ()> {
    if let Some((base, alpha)) = arg.rsplit_once('/') {
        let alpha = (opacity_arg(alpha)? * 255.0).round();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "opacity_arg restricts rounded alpha to 0..=255"
        )]
        let alpha = alpha as u8;
        return match color_arg(base)? {
            PaintColor::Rgba(mut color) => {
                color[3] = u8::try_from((u16::from(color[3]) * u16::from(alpha) + 127) / 255)
                    .map_err(|_| ())?;
                Ok(PaintColor::Rgba(color))
            }
            PaintColor::Token(name) => Ok(PaintColor::TokenAlpha(name, alpha)),
            PaintColor::TokenAlpha(..) => Err(()),
        };
    }
    if let Some(inner) = bracket(arg) {
        let hex = inner.strip_prefix('#').unwrap_or(inner);
        return Ok(PaintColor::Rgba(parse_hex(hex)?));
    }
    if let Some(hex) = arg.strip_prefix('#') {
        return Ok(PaintColor::Rgba(parse_hex(hex)?));
    }
    if !arg.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err(());
    }
    Ok(PaintColor::token(arg))
}

fn parse_hex(hex: &str) -> Result<[u8; 4], ()> {
    let byte = |index: usize| -> Result<u8, ()> {
        let chars: Vec<char> = hex.chars().collect();
        let hi = chars.get(index).copied().ok_or(())?;
        let lo = chars.get(index + 1).copied().ok_or(())?;
        let hi = hi.to_digit(16).ok_or(())?;
        let lo = lo.to_digit(16).ok_or(())?;
        u8::try_from(hi * 16 + lo).map_err(|_| ())
    };
    match hex.len() {
        6 => Ok([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => Err(()),
    }
}

fn bracket(arg: &str) -> Option<&str> {
    arg.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
}

fn parse_f32(text: &str) -> Result<f32, ()> {
    let value: f32 = text.parse().map_err(|_| ())?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(())
    }
}
