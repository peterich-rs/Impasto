//! Compositing layers. The types live in `protocol` so the renderer can read them.

pub use protocol::{Layer, LayerId, LayerKind, LayerTree, PictureId};

use crate::NodeId;

/// Bit 63 marks a node's own picture. Generation uses 21 bits of the high half.
pub const SELF_PICTURE: u64 = 1 << 63;

/// Bit 62 is reserved for a future selection picture. It is unused.
#[allow(dead_code)]
pub const SELECTION_PICTURE: u64 = 1 << 62;

pub fn child_picture_id(id: NodeId) -> PictureId {
    PictureId(id.to_raw())
}

pub fn self_picture_id(id: NodeId) -> PictureId {
    PictureId(id.to_raw() | SELF_PICTURE)
}
