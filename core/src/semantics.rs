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

impl SemanticsId {
    pub fn from_node(id: NodeId) -> Self {
        Self(id.to_raw())
    }
}
