//! Domain values to Taffy. Parsing of arbitrary grid syntax stays here.

use std::str::FromStr;

use taffy::prelude::*;
use taffy::style::{BoxSizing, GridTemplateComponent, GridTemplateTracks, TrackSizingFunction};

use super::StyleError;
use super::environment::{Environment, ResolvedLength};
use super::property::Property;
use super::values::{
    BasisValue, BoxSizingMode, ContentAlign, DirectionMode, DisplayMode, FlexDir, FlexWrapMode,
    GridFlow, GridPlacement, GridTrack, InsetValue, ItemAlign, Length, LimitValue, MarginValue,
    OverflowMode, PositionMode, SizeValue, TrackBound, TrackList,
};

/// Reject a grid source Taffy cannot parse. Bare `0` in a length position
/// becomes `0px` first. Repeat counts and line numbers are left alone.
pub fn accept_track_source(source: &str) -> Result<(), StyleError> {
    parse_track_source(source).map(|_| ())
}

/// Parsed tracks ready to assign to a Taffy style field.
pub fn parse_track_source(source: &str) -> Result<Vec<GridTemplateComponent<String>>, StyleError> {
    let normalized = normalize_track_zeros(source);
    let mut tracks = Vec::new();
    for piece in split_top_level(&normalized) {
        if let Ok(component) = GridTemplateComponent::<String>::from_str(piece) {
            tracks.push(component);
            continue;
        }
        let parsed =
            GridTemplateTracks::<String, TrackSizingFunction>::from_str(piece).map_err(|_| {
                StyleError::Invalid {
                    node: None,
                    property: "gridTemplateColumns",
                    reason: "grid track syntax is not supported",
                }
            })?;
        tracks.extend(parsed.tracks.into_iter().map(GridTemplateComponent::Single));
    }
    if tracks.is_empty() {
        return Err(StyleError::Invalid {
            node: None,
            property: "gridTemplateColumns",
            reason: "track list is empty",
        });
    }
    Ok(tracks)
}

fn split_top_level(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ' ' | '\n' | '\t' if depth == 0 => {
                if start < index {
                    parts.push(&input[start..index]);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    if start < input.len() {
        let tail = input[start..].trim();
        if !tail.is_empty() {
            parts.push(tail);
        }
    }
    parts
}

/// Rewrite a bare `0` that occupies a length position into `0px`.
///
/// `repeat` counts and grid line numbers are not lengths. `0px`, `0%`, and
/// `0fr` already carry a unit and are left alone.
pub fn normalize_track_zeros(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;
    let mut repeat_depth = 0i32;
    let mut seen_comma_in_repeat = false;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"repeat") {
            let after = index + 6;
            let rest = input[after..].trim_start();
            if rest.starts_with('(') {
                out.push_str(&input[index..after]);
                index = after;
                repeat_depth += 1;
                seen_comma_in_repeat = false;
                continue;
            }
        }
        let ch = bytes[index] as char;
        if ch == ',' && repeat_depth > 0 && !seen_comma_in_repeat {
            seen_comma_in_repeat = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '(' && repeat_depth > 0 {
            // nested function inside the track list still counts as length context
        }
        if ch == ')' && repeat_depth > 0 {
            repeat_depth -= 1;
            seen_comma_in_repeat = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '0' && is_bare_zero(bytes, index) {
            let in_repeat_count = repeat_depth > 0 && !seen_comma_in_repeat;
            out.push('0');
            index += 1;
            if !in_repeat_count {
                out.push_str("px");
            }
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// Write one layout longhand onto a Taffy style. Paint-only properties return `Ok(false)`.
pub fn apply_layout(
    style: &mut Style,
    property: &Property,
    env: &Environment,
    rtl: bool,
) -> Result<bool, StyleError> {
    match property {
        Property::Width(value) => style.size.width = size_dim(value, env)?,
        Property::Height(value) => style.size.height = size_dim(value, env)?,
        Property::MinWidth(value) => style.min_size.width = limit_dim(value, env)?,
        Property::MinHeight(value) => style.min_size.height = limit_dim(value, env)?,
        Property::MaxWidth(value) => style.max_size.width = limit_dim(value, env)?,
        Property::MaxHeight(value) => style.max_size.height = limit_dim(value, env)?,
        Property::FlexGrow(value) => style.flex_grow = *value,
        Property::FlexShrink(value) => style.flex_shrink = *value,
        Property::FlexBasis(value) => style.flex_basis = basis_dim(value, env)?,
        Property::PaddingTop(value) => style.padding.top = gap_len(value, env)?,
        Property::PaddingRight(value) => style.padding.right = gap_len(value, env)?,
        Property::PaddingBottom(value) => style.padding.bottom = gap_len(value, env)?,
        Property::PaddingLeft(value) => style.padding.left = gap_len(value, env)?,
        Property::PaddingInlineStart(value) => {
            let edge = gap_len(value, env)?;
            if rtl {
                style.padding.right = edge;
            } else {
                style.padding.left = edge;
            }
        }
        Property::PaddingInlineEnd(value) => {
            let edge = gap_len(value, env)?;
            if rtl {
                style.padding.left = edge;
            } else {
                style.padding.right = edge;
            }
        }
        Property::MarginTop(value) => style.margin.top = margin_dim(value, env)?,
        Property::MarginRight(value) => style.margin.right = margin_dim(value, env)?,
        Property::MarginBottom(value) => style.margin.bottom = margin_dim(value, env)?,
        Property::MarginLeft(value) => style.margin.left = margin_dim(value, env)?,
        Property::MarginInlineStart(value) => {
            let edge = margin_dim(value, env)?;
            if rtl {
                style.margin.right = edge;
            } else {
                style.margin.left = edge;
            }
        }
        Property::MarginInlineEnd(value) => {
            let edge = margin_dim(value, env)?;
            if rtl {
                style.margin.left = edge;
            } else {
                style.margin.right = edge;
            }
        }
        Property::RowGap(value) => style.gap.height = gap_len(value, env)?,
        Property::ColumnGap(value) => style.gap.width = gap_len(value, env)?,
        Property::Display(value) => {
            style.display = match value {
                DisplayMode::Flex => Display::Flex,
                DisplayMode::Grid => Display::Grid,
                DisplayMode::Block => Display::Block,
                DisplayMode::FlowRoot => Display::FlowRoot,
                DisplayMode::None => Display::None,
            }
        }
        Property::FlexDirection(value) => {
            style.flex_direction = match value {
                FlexDir::Row => FlexDirection::Row,
                FlexDir::Column => FlexDirection::Column,
                FlexDir::RowReverse => FlexDirection::RowReverse,
                FlexDir::ColumnReverse => FlexDirection::ColumnReverse,
            }
        }
        Property::FlexWrap(value) => {
            style.flex_wrap = match value {
                FlexWrapMode::NoWrap => FlexWrap::NoWrap,
                FlexWrapMode::Wrap => FlexWrap::Wrap,
                FlexWrapMode::WrapReverse => FlexWrap::WrapReverse,
            }
        }
        Property::AlignItems(value) => style.align_items = item_align(*value)?,
        Property::AlignSelf(value) => style.align_self = Some(item_align(*value)?),
        Property::JustifyItems(value) => style.justify_items = item_align(*value)?,
        Property::JustifySelf(value) => style.justify_self = Some(item_align(*value)?),
        Property::AlignContent(value) => style.align_content = content_align(*value),
        Property::JustifyContent(value) => style.justify_content = content_align(*value),
        Property::Position(value) => {
            style.position = match value {
                PositionMode::Relative => Position::Relative,
                PositionMode::Absolute => Position::Absolute,
            }
        }
        Property::Top(value) => style.inset.top = inset_dim(value, env)?,
        Property::Right(value) => style.inset.right = inset_dim(value, env)?,
        Property::Bottom(value) => style.inset.bottom = inset_dim(value, env)?,
        Property::Left(value) => style.inset.left = inset_dim(value, env)?,
        Property::InsetInlineStart(value) => {
            let edge = inset_dim(value, env)?;
            if rtl {
                style.inset.right = edge;
            } else {
                style.inset.left = edge;
            }
        }
        Property::InsetInlineEnd(value) => {
            let edge = inset_dim(value, env)?;
            if rtl {
                style.inset.left = edge;
            } else {
                style.inset.right = edge;
            }
        }
        Property::AspectRatio(value) => style.aspect_ratio = Some(*value),
        Property::BoxSizing(value) => {
            style.box_sizing = match value {
                BoxSizingMode::BorderBox => BoxSizing::BorderBox,
                BoxSizingMode::ContentBox => BoxSizing::ContentBox,
            }
        }
        Property::BorderWidth(value) => {
            let edge = gap_len(value, env)?;
            style.border = Rect {
                top: edge,
                right: edge,
                bottom: edge,
                left: edge,
            };
        }
        Property::Overflow(value) | Property::OverflowX(value) | Property::OverflowY(value) => {
            let mode = overflow_mode(*value);
            match property {
                Property::Overflow(_) => style.overflow = taffy::Point { x: mode, y: mode },
                Property::OverflowX(_) => style.overflow.x = mode,
                _ => style.overflow.y = mode,
            }
        }
        Property::LayoutDirection(value) => {
            style.direction = match value {
                DirectionMode::Ltr => taffy::style::Direction::Ltr,
                DirectionMode::Rtl => taffy::style::Direction::Rtl,
            }
        }
        Property::GridTemplateColumns(value) => {
            style.grid_template_columns = track_list(value, env)?;
        }
        Property::GridTemplateRows(value) => style.grid_template_rows = track_list(value, env)?,
        Property::GridAutoColumns(value) => style.grid_auto_columns = auto_tracks(value, env)?,
        Property::GridAutoRows(value) => style.grid_auto_rows = auto_tracks(value, env)?,
        Property::GridAutoFlow(value) => {
            style.grid_auto_flow = match value {
                GridFlow::Row => GridAutoFlow::Row,
                GridFlow::Column => GridAutoFlow::Column,
                GridFlow::RowDense => GridAutoFlow::RowDense,
                GridFlow::ColumnDense => GridAutoFlow::ColumnDense,
            }
        }
        Property::GridColumnStart(value) => style.grid_column.start = placement(*value),
        Property::GridColumnEnd(value) => style.grid_column.end = placement(*value),
        Property::GridRowStart(value) => style.grid_row.start = placement(*value),
        Property::GridRowEnd(value) => style.grid_row.end = placement(*value),
        _ => return Ok(false),
    }
    Ok(true)
}

fn size_dim(value: &SizeValue, env: &Environment) -> Result<Dimension, StyleError> {
    Ok(match value {
        SizeValue::Auto => Dimension::auto(),
        SizeValue::MinContent => Dimension::min_content(),
        SizeValue::MaxContent => Dimension::max_content(),
        SizeValue::FitContent => Dimension::fit_content(),
        SizeValue::Stretch => Dimension::stretch(),
        SizeValue::Length(length) => length_dim(length, env)?,
        SizeValue::FitContentLimit(length) => match env.resolve_length(length)? {
            ResolvedLength::Points(points) => Dimension::fit_content_px(points),
            ResolvedLength::Fraction(fraction) => Dimension::fit_content_percent(fraction),
        },
    })
}

fn length_dim(value: &Length, env: &Environment) -> Result<Dimension, StyleError> {
    Ok(match env.resolve_length(value)? {
        ResolvedLength::Points(points) => length(points),
        ResolvedLength::Fraction(fraction) => percent(fraction),
    })
}

fn limit_dim(value: &LimitValue, env: &Environment) -> Result<LengthPercentageAuto, StyleError> {
    Ok(match value {
        LimitValue::Auto => LengthPercentageAuto::auto(),
        LimitValue::Length(value) => match env.resolve_length(value)? {
            ResolvedLength::Points(points) => length(points),
            ResolvedLength::Fraction(fraction) => percent(fraction),
        },
    })
}

fn basis_dim(value: &BasisValue, env: &Environment) -> Result<Dimension, StyleError> {
    Ok(match value {
        BasisValue::Content => Dimension::content(),
        BasisValue::Size(size) => size_dim(size, env)?,
    })
}

fn gap_len(value: &Length, env: &Environment) -> Result<LengthPercentage, StyleError> {
    Ok(match env.resolve_length(value)? {
        ResolvedLength::Points(points) => length(points),
        ResolvedLength::Fraction(fraction) => percent(fraction),
    })
}

fn margin_dim(value: &MarginValue, env: &Environment) -> Result<LengthPercentageAuto, StyleError> {
    Ok(match value {
        MarginValue::Auto => LengthPercentageAuto::auto(),
        MarginValue::Length(value) => match env.resolve_length(value)? {
            ResolvedLength::Points(points) => length(points),
            ResolvedLength::Fraction(fraction) => percent(fraction),
        },
    })
}

fn inset_dim(value: &InsetValue, env: &Environment) -> Result<LengthPercentageAuto, StyleError> {
    margin_dim(
        &match value {
            InsetValue::Auto => MarginValue::Auto,
            InsetValue::Length(length) => MarginValue::Length(length.clone()),
        },
        env,
    )
}

fn item_align(value: ItemAlign) -> Result<AlignItems, StyleError> {
    Ok(match value {
        ItemAlign::Auto => AlignItems::START,
        ItemAlign::Normal => AlignItems::NORMAL,
        ItemAlign::Start => AlignItems::START,
        ItemAlign::End => AlignItems::END,
        ItemAlign::FlexStart => AlignItems::FLEX_START,
        ItemAlign::FlexEnd => AlignItems::FLEX_END,
        ItemAlign::Center => AlignItems::CENTER,
        ItemAlign::Stretch => AlignItems::STRETCH,
        ItemAlign::Baseline => AlignItems::BASELINE,
    })
}

fn content_align(value: ContentAlign) -> JustifyContent {
    match value {
        ContentAlign::Start => JustifyContent::START,
        ContentAlign::End => JustifyContent::END,
        ContentAlign::FlexStart => JustifyContent::FLEX_START,
        ContentAlign::FlexEnd => JustifyContent::FLEX_END,
        ContentAlign::Center => JustifyContent::CENTER,
        ContentAlign::Stretch => JustifyContent::STRETCH,
        ContentAlign::SpaceBetween => JustifyContent::SPACE_BETWEEN,
        ContentAlign::SpaceAround => JustifyContent::SPACE_AROUND,
        ContentAlign::SpaceEvenly => JustifyContent::SPACE_EVENLY,
    }
}

fn overflow_mode(value: OverflowMode) -> taffy::style::Overflow {
    match value {
        OverflowMode::Visible => taffy::style::Overflow::Visible,
        OverflowMode::Clip => taffy::style::Overflow::Clip,
        OverflowMode::Hidden => taffy::style::Overflow::Hidden,
    }
}

fn placement(value: GridPlacement) -> taffy::style::GridPlacement<String> {
    match value {
        GridPlacement::Auto => taffy::style::GridPlacement::Auto,
        GridPlacement::Line(line) => taffy::style::GridPlacement::Line(line.into()),
        GridPlacement::Span(count) => taffy::style::GridPlacement::Span(count),
    }
}

fn track_list(
    tracks: &TrackList,
    env: &Environment,
) -> Result<Vec<GridTemplateComponent<String>>, StyleError> {
    match tracks {
        TrackList::Source(source) => parse_track_source(source),
        TrackList::Tracks(tracks) => tracks
            .iter()
            .map(|track| domain_track(track, env))
            .collect(),
    }
}

fn auto_tracks(
    tracks: &TrackList,
    env: &Environment,
) -> Result<Vec<TrackSizingFunction>, StyleError> {
    let list = track_list(tracks, env)?;
    let mut out = Vec::new();
    for component in list {
        if let GridTemplateComponent::Single(track) = component {
            out.push(track);
        }
    }
    Ok(out)
}

fn domain_track(
    track: &GridTrack,
    env: &Environment,
) -> Result<GridTemplateComponent<String>, StyleError> {
    let single = match track {
        GridTrack::Bound(bound) => bound_track(*bound, env)?,
        GridTrack::MinMax { min, max } => minmax(bound_min(*min, env)?, bound_max(*max, env)?),
    };
    Ok(GridTemplateComponent::Single(single))
}

fn bound_track(bound: TrackBound, env: &Environment) -> Result<TrackSizingFunction, StyleError> {
    Ok(minmax(bound_min(bound, env)?, bound_max(bound, env)?))
}

fn bound_min(bound: TrackBound, _env: &Environment) -> Result<MinTrackSizingFunction, StyleError> {
    Ok(match bound {
        TrackBound::Auto => min_content(),
        TrackBound::MinContent => min_content(),
        TrackBound::MaxContent => max_content(),
        TrackBound::Points(value) => length(value),
        TrackBound::Percent(value) => percent(value),
        TrackBound::Fraction(_) => length(0.0),
    })
}

fn bound_max(bound: TrackBound, _env: &Environment) -> Result<MaxTrackSizingFunction, StyleError> {
    Ok(match bound {
        TrackBound::Auto => auto(),
        TrackBound::MinContent => min_content(),
        TrackBound::MaxContent => max_content(),
        TrackBound::Points(value) => length(value),
        TrackBound::Percent(value) => percent(value),
        TrackBound::Fraction(value) => fr(value),
    })
}

fn is_bare_zero(bytes: &[u8], index: usize) -> bool {
    if index > 0 {
        let prev = bytes[index - 1];
        if prev.is_ascii_digit() || prev == b'.' {
            return false;
        }
    }
    let mut end = index + 1;
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            if bytes[end] != b'0' {
                return false;
            }
            end += 1;
        }
    }
    if end < bytes.len() {
        let next = bytes[end];
        if next.is_ascii_alphabetic() || next == b'%' || next.is_ascii_digit() {
            return false;
        }
    }
    true
}
