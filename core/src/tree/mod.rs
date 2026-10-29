//! Render tree. Layout goes through Taffy. Positions, hits, and gestures stay on this type.

mod gesture;
mod hit;
mod layout;
mod node;
mod record;
mod scroll;
mod semantics;
mod store;
mod text;

#[cfg(test)]
pub(crate) use hit::Affine;
pub use layout::{LayoutCause, LayoutTrace};
pub use node::{
    Alignment, AxisSize, Discarded, DragClaim, Edges, FocusDirection, Gesture, GridLine, Inset,
    Justify, NodeId, Rect, RowCrossAlign, ScrollEdge, SubtreeSpace, TrackSize,
};

use crate::{AnimProp, Simulation};

struct ActiveMotion {
    started_ms: f64,
    node: NodeId,
    prop: AnimProp,
    simulation: Box<dyn Simulation>,
    /// This spring is the snap that follows coast. Its end must not snap again.
    snap: bool,
}

use std::cell::Cell;
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
    /// Subtrees [`Tree::remove`] detached. [`Tree::discard_detached`] retires
    /// the ones that are still parentless.
    pending_discard: Vec<NodeId>,
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
    /// Child picture of a scroll. Keyed by the scroll node.
    pictures: HashMap<NodeId, Arc<protocol::Picture>>,
    /// Runs keyed by the owner that closed them, in paint order.
    runs: HashMap<NodeId, Vec<Arc<protocol::Picture>>>,
    /// Capture in progress. Empty outside `record_into`.
    run: record::RunState,
    /// Owners whose runs were sealed by the current `record_into`.
    finished_owners: Vec<NodeId>,
    /// Strict descendants that open layers. Recomputed at the start of each record.
    subtree_layers: HashMap<NodeId, bool>,
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
    /// Text nodes whose paint key must be retained before the next record.
    pending_text: Vec<NodeId>,
    sync_visited: u32,
    sync_synced: u32,
    sync_ms: f64,
    taffy_ms: f64,
    /// Sum of `layout` bodies since the last [`Tree::take_layout_trace`].
    layout_work_ms: f64,
    /// `layout` calls that entered Taffy since the last trace.
    layout_entries: u32,
    /// First reason `needs_layout` became true since the last trace.
    layout_cause: LayoutCause,
    text_reused: Cell<u32>,
    text_shaped: Cell<u32>,
    /// `layout_passes` already published by [`Tree::semantics_patch`].
    semantics_layout_seen: u64,
    /// Scroll nodes whose offset changed since the last full semantics walk.
    scroll_semantics: Vec<NodeId>,
    /// Hint or structure changed. The next patch walks the whole tree.
    semantics_full: bool,
    /// Reused by [`Tree::sweep_subtree_layers`].
    sweep_stack: Vec<record::SweepFrame>,
    sweep_children: Vec<NodeId>,
    /// Multiplier applied when measuring and recording text. `1.0` leaves sizes alone.
    text_scale: f32,
    pixel_scale: f32,
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
    fn removed_child_can_be_inserted_until_discard_detached() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.remove(parent, child)?;
        tree.set_text(child, "still")?;
        tree.add_child(parent, child)?;
        assert_eq!(tree.parent(child)?, Some(parent));
        assert!(tree.discard_detached().ids.is_empty());
        let anchor = tree.text("A", 16.0, [0, 0, 0, 255])?;
        let extra = tree.text("B", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, anchor)?;
        tree.remove(parent, anchor)?;
        tree.insert(parent, extra, Some(anchor))?;
        assert_eq!(tree.parent(extra)?, Some(parent));
        tree.remove(parent, child)?;
        tree.remove(parent, child)?;
        let retired = tree.discard_detached();
        assert!(retired.ids.contains(&child));
        assert!(matches!(tree.parent(child), Err(Error::UnknownNode)));
        Ok(())
    }

    #[test]
    fn remove_keeps_taffy_and_the_paragraph() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.layout()?;
        tree.remove(parent, child)?;
        let node = tree.node(child)?;
        assert!(node.taffy.is_some());
        assert!(node.text().and_then(|text| text.paragraph).is_some());
        assert!(tree.detach(parent, child)?.is_empty());
        assert!(tree.pending_discard.is_empty());
        assert!(tree.node(child)?.taffy.is_some());
        let retired = tree.discard(child)?;
        assert!(retired.ids.contains(&child));
        assert!(matches!(tree.node(child), Err(Error::UnknownNode)));
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

    #[test]
    fn revive_rebuilds_taffy_after_release() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let parent = tree.column(0.0)?;
        let child = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        tree.add_child(parent, child)?;
        tree.layout()?;
        tree.detach(parent, child)?;
        tree.release_taffy(child)?;
        assert!(tree.node(child)?.taffy.is_none());
        tree.add_child(parent, child)?;
        assert!(tree.node(child)?.taffy.is_some());
        tree.layout()?;
        assert!(tree.rect(child)?.w > 0.0);
        Ok(())
    }
}
