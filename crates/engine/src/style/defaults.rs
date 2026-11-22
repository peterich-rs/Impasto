//! Constructor presets shared by tree creation and scroll-axis updates.

use taffy::prelude::*;

use crate::motion::ScrollAxis;

pub fn zero_min() -> Size<LengthPercentageAuto> {
    Size {
        width: length(0.0_f32),
        height: length(0.0_f32),
    }
}

/// One grid track that fills its box and cannot grow past it.
pub fn fill_track() -> TrackSizingFunction {
    minmax(length(0.0_f32), fr(1.0_f32))
}

/// Scroll viewport. The scroll axis is max-content; the cross axis is `minmax(0, 1fr)`.
pub fn scroll_style_for(axis: ScrollAxis) -> Style {
    let (columns, rows) = match axis {
        ScrollAxis::Vertical => (
            vec![GridTemplateComponent::Single(fill_track())],
            vec![max_content()],
        ),
        ScrollAxis::Horizontal => (
            vec![max_content()],
            vec![GridTemplateComponent::Single(fill_track())],
        ),
        ScrollAxis::Both => (vec![max_content()], vec![max_content()]),
    };
    Style {
        display: Display::Grid,
        grid_template_columns: columns,
        grid_template_rows: rows,
        align_items: AlignItems::STRETCH,
        justify_items: AlignItems::STRETCH,
        align_self: Some(AlignSelf::STRETCH),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: zero_min(),
        overflow: taffy::Point {
            x: taffy::style::Overflow::Hidden,
            y: taffy::style::Overflow::Hidden,
        },
        ..content_box()
    }
}

/// Stack, clip, opacity, and transform. One min-content cell.
pub fn stack_style() -> Style {
    Style {
        display: Display::Grid,
        flex_shrink: 0.0,
        grid_template_columns: vec![min_content()],
        grid_template_rows: vec![min_content()],
        ..content_box()
    }
}

/// New `View`. Column, min 0, grow 0, shrink 0, basis auto, no implicit max.
pub fn view_style() -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        flex_grow: 0.0,
        flex_shrink: 0.0,
        min_size: zero_min(),
        align_items: AlignItems::STRETCH,
        justify_content: JustifyContent::FLEX_START,
        ..content_box()
    }
}

/// Leaf and content box. Shrink stays off until explicitly enabled.
pub fn content_box() -> Style {
    Style {
        // Every scene node remains its children's containing block.
        position: Position::Relative,
        flex_shrink: 0.0,
        ..Style::default()
    }
}

/// Row or column preset with the same defaults as View.
pub fn flex_style(direction: FlexDirection, gap: f32) -> Style {
    Style {
        flex_direction: direction,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..view_style()
    }
}
