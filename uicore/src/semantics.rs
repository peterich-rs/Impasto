//! Accessibility tree produced from a laid-out render tree.

use crate::tree::{NodeId, Rect};

/// Stable id for one semantics node. The value is [`NodeId::to_raw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SemanticsId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticsRole {
    Generic,
    Button,
    Text,
    Image,
    Scroll,
    Switch,
    TextField,
    SearchField,
    TextArea,
}

/// How a live region announces changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveRegion {
    Off,
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SemanticsActions(pub u32);

impl SemanticsActions {
    pub const ACTIVATE: u32 = 1;
    pub const SCROLL: u32 = 2;
    pub const SET_TEXT: u32 = 4;

    pub const fn contains(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SemanticsFlags(pub u32);

impl SemanticsFlags {
    pub const FOCUSABLE: u32 = 1;
    pub const CHECKED: u32 = 2;
    pub const DISABLED: u32 = 4;
    pub const EDITABLE: u32 = 8;
    pub const READ_ONLY: u32 = 16;

    pub const fn contains(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticsNode {
    pub id: SemanticsId,
    pub parent: Option<SemanticsId>,
    pub rect: Rect,
    pub role: SemanticsRole,
    pub label: String,
    pub value: Option<String>,
    pub actions: SemanticsActions,
    pub flags: SemanticsFlags,
    /// Scroll offset in scene points, when `role` is [`SemanticsRole::Scroll`].
    pub scroll: Option<(f32, f32)>,
    /// UTF-16 selection inside [`SemanticsNode::value`].
    pub selection: Option<(u32, u32)>,
    /// Collapsed caret, UTF-16 extent of the focused editor. Other nodes leave this empty.
    pub caret: Option<u32>,
    /// Extra description, such as a field error. Never the password text.
    pub description: Option<String>,
}

/// Optional override of the inferred semantics for one node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticsHint {
    pub role: Option<SemanticsRole>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub selection: Option<(u32, u32)>,
    pub disabled: Option<bool>,
    pub editable: Option<bool>,
    pub read_only: Option<bool>,
    pub description: Option<String>,
    /// Focus traversal weight. Missing keeps tree order.
    pub order: Option<i32>,
    pub live: Option<LiveRegion>,
}

/// An action coming back from the platform accessibility bridge.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticsAction {
    Activate(SemanticsId),
    Scroll { id: SemanticsId, dx: f32, dy: f32 },
    SetText { id: SemanticsId, text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticsUpdate {
    pub nodes: Vec<SemanticsNode>,
    pub root: SemanticsId,
}

/// One change since the previous semantics patch.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticsOp {
    Upsert(SemanticsNode),
    Remove(SemanticsId),
}

/// Nodes that changed. An empty `ops` is not produced; the session uses `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticsPatch {
    pub root: SemanticsId,
    pub ops: Vec<SemanticsOp>,
}

/// Live image sources. `generation` bumps on insert and removal, not on `Arc::strong_count`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageCensus {
    pub generation: u64,
    pub counts: std::sync::Arc<std::collections::HashMap<std::sync::Arc<str>, u32>>,
}

impl SemanticsId {
    pub fn from_node(id: NodeId) -> Self {
        Self(id.to_raw())
    }
}
