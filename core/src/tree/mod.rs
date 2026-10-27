//! Render tree. Layout goes through Taffy. Positions, hits, and gestures stay on this type.

mod gesture;
mod hit;
mod layout;
mod node;
mod record;
mod semantics;
mod store;

pub use node::{
    Alignment, AxisSize, Discarded, DragClaim, Edges, FocusDirection, Gesture, GridLine, Inset,
    Justify, NodeId, Rect, RowCrossAlign, ScrollEdge, TrackSize,
};

use crate::{AnimProp, Simulation};

struct ActiveMotion {
    started_ms: f64,
    node: NodeId,
    prop: AnimProp,
    simulation: Box<dyn Simulation>,
}

use std::collections::HashMap;
use std::sync::Arc;

use node::Slot;
use taffy::TaffyTree;

#[cfg(test)]
use node::MAX_GENERATION;

use crate::Shaper;

/// Render tree. `layout` writes positions. `pointer` updates click and drag.
///
/// The shaper is erased here so one tree type serves the test shaper and the
/// host's Impeller shaper. The trait is not `Send`: an Impeller typography
/// context stays on the UI thread, and so does the tree.
pub struct Tree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    taffy: TaffyTree<usize>,
    shaper: Box<dyn Shaper>,
    root: Option<NodeId>,
    /// Press, drag, and hover routes. Structure edits do not clear this.
    arena: gesture::GestureArena,
    focused: Option<NodeId>,
    /// Innermost focus scope. Empty means the whole tree.
    scopes: Vec<NodeId>,
    /// Scroll friction and property springs, sampled by [`Tree::tick_motions`].
    motions: Vec<ActiveMotion>,
    /// Motions that finished since the last [`Tree::take_motion_ends`].
    motion_ends: Vec<(NodeId, AnimProp)>,
    /// Last timestamp passed to [`Tree::tick_motions`]. New motions start here.
    clock_ms: f64,
    /// Set while a sample writes a property, so that write does not rebase itself.
    applying_motion: bool,
    /// Relayout boundaries whose children must be read back from Taffy.
    dirty_boundaries: Vec<NodeId>,
    /// Child commands for a repaint boundary. Replay skips the subtree.
    pictures: HashMap<NodeId, Arc<protocol::Picture>>,
    /// Commands for the boundary node itself. Not the child picture's slot.
    self_pictures: HashMap<NodeId, Arc<protocol::Picture>>,
    /// Parent layer while `record` is linking the layer tree.
    layer_stack: Vec<protocol::LayerId>,
    /// Last sibling linked under each layer, indexed by [`protocol::LayerId`].
    layer_tail: Vec<Option<protocol::LayerId>>,
    /// Last layer linked with no parent. Its `next` is the following top-level layer.
    layer_top: Option<protocol::LayerId>,
    /// Commands written into pictures by the latest `record`.
    picture_writes: usize,
    /// Bumps when a picture is rebuilt or a layer property changes.
    layer_epoch: u64,
    layers: crate::layer::LayerTree,
    /// Last semantics nodes. A clean node reuses its strings.
    semantics_cache: HashMap<crate::SemanticsId, crate::SemanticsNode>,
    /// Image sources still on the tree. Maintained on insert and removal.
    image_counts: HashMap<Arc<str>, u32>,
    image_generation: u64,
    /// Offset notices produced by [`Tree::tick_motions`], drained by the session.
    scroll_notices: Vec<Gesture>,
    /// Style or structure must run Taffy again.
    needs_layout: bool,
    /// Paint changed. Layout boxes stay valid.
    needs_paint: bool,
    /// How many times `layout` entered Taffy. Paint-only updates leave this unchanged.
    layout_passes: u64,
    /// Multiplier applied when measuring and recording text. `1.0` leaves sizes alone.
    text_scale: f32,
    laid_out: bool,
    /// Content origin in scene points. Insets shift the root; `(0, 0)` is the surface origin.
    origin: (f32, f32),
    viewport: Option<(f32, f32)>,
}

#[cfg(test)]
mod slot_tests {
    use super::{MAX_GENERATION, Tree};
    use crate::{Error, MonoShaper};

    #[test]
    fn recycled_slot_rejects_the_previous_id() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let stale = text;
        tree.retire(text.index);
        let reused = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_eq!(reused.index, stale.index);
        assert!(matches!(
            tree.set_text(stale, "no"),
            Err(Error::UnknownNode)
        ));
        tree.set_text(reused, "Ok")?;
        Ok(())
    }

    #[test]
    fn generation_cap_retires_the_slot() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let index = text.index;
        tree.slots[index as usize].generation = MAX_GENERATION;
        tree.retire(index);
        assert!(tree.free.is_empty());
        let next = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_ne!(next.index, index);
        Ok(())
    }
}
