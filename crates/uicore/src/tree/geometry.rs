//! Content box shared by layout sync. Effect order lives with the recorder.

use taffy::prelude::Layout;

/// Border plus padding, in the node's parent space. Text and images start here.
pub(super) fn content_box(layout: &Layout) -> (f32, f32, f32, f32) {
    (
        layout.border.left + layout.padding.left,
        layout.border.top + layout.padding.top,
        layout.content_box_width().max(0.0),
        layout.content_box_height().max(0.0),
    )
}
