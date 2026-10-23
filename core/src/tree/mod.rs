//! Render tree. Layout goes through Taffy. Positions, hits, and gestures stay on this type.

mod gesture;
mod hit;
mod layout;
mod node;
mod record;
mod store;

pub use node::{Alignment, Discarded, Edges, Gesture, Justify, NodeId, Rect, TrackSize};

use gesture::{DragState, SecondaryPress};
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
    pressed: Option<NodeId>,
    secondary: Option<SecondaryPress>,
    focused: Option<NodeId>,
    hovered: Option<NodeId>,
    /// Leave gestures produced by a structure change, delivered on the next pointer or remove.
    pending: Vec<Gesture>,
    drag: Option<DragState>,
    dirty: bool,
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
