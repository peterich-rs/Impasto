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
    Justify, NodeId, Rect, TrackSize,
};

use std::collections::HashMap;

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
    /// Scroll offsets still coasting. Values are scene points per millisecond.
    fling: HashMap<NodeId, (f32, f32)>,
    /// Offset notices produced by [`Tree::tick_fling`], drained by the session.
    scroll_notices: Vec<Gesture>,
    /// Per-node editors. Field bindings call in; the tree does not paint from this map.
    editors: HashMap<NodeId, crate::text::TextEditor>,
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
        let mut tree = Tree::new(MonoShaper);
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
        let mut tree = Tree::new(MonoShaper);
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
