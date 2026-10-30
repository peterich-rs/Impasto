//! Window, thumb, and pull arithmetic. Same inputs as the old list-window module.

use super::prefix::PrefixHeights;

pub const REFRESH_THRESHOLD: f32 = 64.0;
pub const MIN_THUMB: f32 = 24.0;
pub const FLING_VIEW_FACTOR: f32 = 3.0;
pub const HEIGHT_EPS: f32 = 0.5;

/// Finite and positive, otherwise 1.
pub fn estimate_height(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

/// `thumb = max(24, viewport² / content)`.
pub fn thumb_length(viewport: f32, content: f32) -> f32 {
    if content <= 0.0 || viewport <= 0.0 {
        return MIN_THUMB;
    }
    MIN_THUMB.max((viewport * viewport) / content)
}

/// `travel * offset / max(content - viewport, 1)`.
pub fn thumb_offset(viewport: f32, content: f32, offset: f32) -> f32 {
    let thumb = thumb_length(viewport, content);
    let travel = (viewport - thumb).max(0.0);
    (travel * offset) / (content - viewport).max(1.0)
}

/// No next header stays at 0. Otherwise `min(0, nextStart - offset - headerHeight)`.
pub fn sticky_translate(next_start: Option<f32>, offset: f32, header_height: f32) -> f32 {
    let Some(next_start) = next_start else {
        return 0.0;
    };
    (next_start - offset - header_height).min(0.0)
}

/// Finger position, including distance already past the edge.
pub fn drag_origin(
    offset_x: f32,
    offset_y: f32,
    overscroll_x: f32,
    overscroll_y: f32,
    factor: f32,
) -> (f32, f32) {
    if factor <= 0.0 {
        return (offset_x, offset_y);
    }
    (
        offset_x + overscroll_x / factor,
        offset_y + overscroll_y / factor,
    )
}

/// Only the top edge adds `next_y - raw_y`.
pub fn pull_delta(live_y: f32, raw_y: f32, next_y: f32) -> f32 {
    if live_y <= 0.0 && raw_y < 0.0 {
        next_y - raw_y
    } else {
        0.0
    }
}

/// Window tail is within `overscan` of `length`.
pub fn near_end(end: usize, length: usize, overscan: usize) -> bool {
    length.saturating_sub(end) < overscan
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRange {
    pub start: usize,
    pub end: usize,
    pub top: f32,
    pub bottom: f32,
}

impl WindowRange {
    pub const EMPTY: Self = Self {
        start: 0,
        end: 0,
        top: 0.0,
        bottom: 0.0,
    };
}

/// Visible slice plus overscan. `end` is exclusive.
pub fn window_range(
    prefix: &PrefixHeights,
    offset: f32,
    view_height: f32,
    overscan: usize,
) -> WindowRange {
    let count = prefix.len();
    if count == 0 {
        return WindowRange::EMPTY;
    }
    let mut start = prefix.find_index(offset);
    if start == count {
        return WindowRange {
            start: count,
            end: count,
            top: prefix.total(),
            bottom: 0.0,
        };
    }
    let mut end = start;
    let limit = offset + view_height.max(0.0);
    while end < count && prefix.item_start(end) < limit {
        end += 1;
    }
    start = start.saturating_sub(overscan);
    end = (end + overscan).min(count);
    WindowRange {
        start,
        end,
        top: prefix.item_start(start),
        bottom: prefix.total() - prefix.item_start(end),
    }
}

/// Expand immediately. Shrink only after the viewport has moved past `overscan` rows.
pub fn stabilize_window(
    prefix: &PrefixHeights,
    current: WindowRange,
    next: WindowRange,
    overscan: usize,
) -> WindowRange {
    let start = if next.start > current.start.saturating_add(overscan) {
        next.start
    } else {
        current.start.min(next.start)
    };
    let end = if next.end.saturating_add(overscan) < current.end {
        next.end
    } else {
        current.end.max(next.end)
    };
    WindowRange {
        start,
        end,
        top: prefix.item_start(start),
        bottom: prefix.total() - prefix.item_start(end),
    }
}

/// Same window as [`window_range`], widened toward `fling_y` while a fling is in flight.
pub fn window_range_for_offset(
    prefix: &PrefixHeights,
    offset: f32,
    view_height: f32,
    overscan: usize,
    fling_y: Option<f32>,
) -> WindowRange {
    let view = view_height.max(0.0);
    let Some(fling_y) = fling_y.filter(|value| value.is_finite()) else {
        return window_range(prefix, offset, view, overscan);
    };
    let cap = FLING_VIEW_FACTOR * view;
    let mut start = offset.min(fling_y);
    let mut end = (offset + view).max(fling_y);
    if end - start > cap {
        if fling_y >= offset {
            start = offset;
            end = offset + cap;
        } else {
            end = offset + view;
            start = (end - cap).max(0.0);
        }
    }
    window_range(prefix, start, (end - start).max(0.0), overscan)
}

/// The current window still covers the viewport, with a slack band.
pub fn window_covers(
    prefix: &PrefixHeights,
    range: WindowRange,
    offset_y: f32,
    view_height: f32,
) -> bool {
    if range.end <= range.start {
        return false;
    }
    let top = prefix.item_start(range.start);
    let bottom = prefix.item_start(range.end);
    let view = view_height.max(0.0);
    let slack = (view * 0.25).min(((bottom - top - view).max(0.0)) * 0.5);
    let view_bottom = offset_y + view;
    offset_y >= top + slack && view_bottom <= bottom - slack
}
