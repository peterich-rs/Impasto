//! Render tree. Layout goes through Taffy. This crate does not link Impeller.

mod cast;
mod error;
mod gesture;
mod layer;
mod list;
mod motion;
mod pointer;
mod semantics;
mod shaper;
mod text;
mod tree;

#[cfg(test)]
mod barrier_test;
#[cfg(test)]
mod layout_test;
#[cfg(test)]
mod refactor_test;
#[cfg(test)]
mod scroll_test;

pub use error::Error;
pub use gesture::{
    DRAG_SLOP, DoubleTapConfig, DragAxis, HitEntry, HitPath, LONG_PRESS_MS, RecognizerSet,
    VELOCITY_WINDOW_MS, VelocityRing,
};
pub use layer::LayerTree;
pub use list::{
    Chrome, DriverEvent, DriverId, DriverSet, ListItem, ListNodes, ListOp, ScrollConfig,
};
pub use motion::{
    AnimProp, Band, Decay, Friction, OverscrollMode, PullMode, SCROLL_TAU_MS, ScrollAccept,
    ScrollAxis, ScrollPhase, ScrollPhysics, Simulation, SnapAlign, SnapGrid, Spring,
};
pub use pointer::{Button, Buttons, Modifiers, Phase, PointerEvent, PointerId, PointerKind};
pub use protocol::ParagraphId;
pub use semantics::{
    ImageCensus, LiveRegion, SemanticsAction, SemanticsActions, SemanticsFlags, SemanticsHint,
    SemanticsId, SemanticsNode, SemanticsOp, SemanticsPatch, SemanticsRole, SemanticsUpdate,
};
pub use shaper::{
    Affinity, Caret, MetricsKey, MonoShaper, PaintKey, ParagraphMetrics, SINGLE_LINE_MAX, Shaper,
    SpanMetrics, TextLine, TextSize, paint_key, resolved_line_height,
};
pub use text::{
    Edit, TextEditingValue, TextEditor, TextSelection, grapheme_count, grapheme_index,
    grapheme_unit,
};
pub use tree::{
    Alignment, AxisSize, BoxEdge, DebugAxis, Discarded, DragClaim, Edges, FocusDirection, Gesture,
    GridLine, Inset, Justify, LayoutCause, LayoutIssue, LayoutTrace, NodeId, PanelAlign, Rect,
    RowCrossAlign, ScrollEdge, ScrollSample, SubtreeSpace, TrackSize, Tree,
};
