//! Render tree. Layout goes through Taffy. This crate does not link Impeller.

mod error;
mod pointer;
mod semantics;
mod shaper;
mod text;
mod tree;

#[cfg(test)]
mod barrier_test;
#[cfg(test)]
mod layout_test;

pub use error::Error;
pub use pointer::{Button, Buttons, Modifiers, Phase, PointerEvent, PointerId, PointerKind};
pub use semantics::{
    SemanticsAction, SemanticsActions, SemanticsFlags, SemanticsHint, SemanticsId, SemanticsNode,
    SemanticsRole, SemanticsUpdate,
};
pub use shaper::{Affinity, Caret, MonoShaper, SINGLE_LINE_MAX, Shaper, TextLine, TextSize};
pub use text::{
    Edit, TextEditingValue, TextEditor, TextSelection, grapheme_count, grapheme_index,
    grapheme_unit,
};
pub use tree::{
    Alignment, AxisSize, Discarded, DragClaim, Edges, FocusDirection, Gesture, GridLine, Inset,
    Justify, NodeId, Rect, TrackSize, Tree,
};
