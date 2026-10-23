//! Node identity, gesture names, and the private node payload.

use std::cell::Cell;

use protocol::TextStyle;
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

pub(super) const MAX_GENERATION: u32 = (1 << 21) - 1;

/// A node in one [`Tree`]. The slot index is stable. `generation` changes when
/// the slot is recycled, so a stale id cannot address the node that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(super) index: u32,
    pub(super) generation: u32,
}

impl NodeId {
    pub fn to_raw(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }

    pub fn from_raw(raw: u64) -> Self {
        Self {
            index: raw as u32,
            generation: (raw >> 32) as u32,
        }
    }

    pub(super) fn slot(self) -> usize {
        self.index as usize
    }
}

/// How a pointer event changed a [`Tree::pointer_region`], a scroll, or focus.
///
/// `DragBy` is a scene-point delta. `DragEnd`'s pair is velocity in scene points
/// per millisecond. A locked drag emits `Release` for the pressed region, then
/// `DragBegin`, then `DragBy` samples, then `DragEnd`. A click moves focus:
/// `Blur` of the previous node, then `Focus` of the focusable ancestor, when
/// that pair actually changes. A drag that locks the scroll does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Press(NodeId),
    Release(NodeId),
    Click(NodeId),
    DragBegin(NodeId),
    DragBy(NodeId, f32, f32),
    DragEnd(NodeId, f32, f32),
    Focus(NodeId),
    Blur(NodeId),
    Hover(NodeId),
    Leave(NodeId),
    /// Wheel deltas in scene points. The id is the nearest scroll ancestor.
    Wheel(NodeId, f32, f32),
    /// Right click on a pointer region. No focus change.
    SecondaryClick(NodeId),
}

/// Ids retired by [`Tree::discard`], children before their parent, plus any
/// focus gesture that retirement produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Discarded {
    pub ids: Vec<NodeId>,
    pub gestures: Vec<Gesture>,
}

/// Main-axis distribution for a row or column. Matches CSS `justify-content`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// How a child sits inside an [`Tree::align`] node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

/// Four edges in scene points. [`Edges::all`] is the uniform padding the older API writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// One grid track. `Points` and `Fraction` must be finite and non-negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackSize {
    MinContent,
    MaxContent,
    Points(f32),
    Fraction(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Row,
    Column,
    Grid,
    Padding,
    Align,
    Sized,
    Stack,
    Clip,
    Opacity,
    Scroll,
    Transform,
    Text,
    Image,
    PointerRegion,
    Decorated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    /// The box itself is a target when no child takes the point.
    Opaque,
    /// Only children can be hit.
    Defer,
    /// The box is a click target. Children are tested first.
    Region,
}

pub(super) struct TextNode {
    pub(super) utf8: String,
    pub(super) font_size: f32,
    pub(super) color: [u8; 4],
    /// Paragraph constraint passed to [`Shaper::measure`] for this layout.
    pub(super) max_width: Cell<f32>,
    pub(super) style: TextStyle,
}

pub(super) struct ImageNode {
    pub(super) source: String,
    pub(super) width: f32,
    pub(super) height: f32,
}

pub(super) struct Node {
    pub(super) kind: Kind,
    /// Copied at creation. Revive and the setters read this, not the live Taffy style.
    pub(super) style: Style,
    /// `None` after the subtree's Taffy nodes were released. The scene node stays.
    pub(super) taffy: Option<TaffyId>,
    pub(super) parent: Option<NodeId>,
    pub(super) children: Vec<NodeId>,
    pub(super) hit: Hit,
    pub(super) text: Option<TextNode>,
    pub(super) image: Option<ImageNode>,
    pub(super) fill: Option<[u8; 4]>,
    /// Corner radius for [`Kind::Decorated`] and [`Kind::Clip`]. Other kinds keep `0.0`.
    pub(super) corner_radius: f32,
    /// Layer alpha for [`Kind::Opacity`].
    pub(super) alpha: Option<u8>,
    /// Paint and hit translation for [`Kind::Scroll`]. Not a layout input.
    pub(super) scroll: (f32, f32),
    pub(super) translate_x: f32,
    pub(super) translate_y: f32,
    pub(super) rotate_rad: f32,
    /// Uniform scale for [`Kind::Transform`]. `1.0` leaves size unchanged.
    pub(super) scale: f32,
    /// Tap-to-focus target. Containers start cleared. Text and images do too.
    pub(super) focusable: bool,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

pub(super) struct Slot {
    pub(super) generation: u32,
    pub(super) node: Option<Node>,
}
