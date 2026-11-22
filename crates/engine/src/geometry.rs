//! Node identity and scene-space geometry shared by input, semantics and the UI tree.

/// Leaves the upper bits of a raw identity available for picture IDs and JS handles.
pub(crate) const MAX_GENERATION: u32 = (1 << 21) - 1;

/// A node in one [`crate::tree::Tree`]. The slot index is stable. `generation` changes when
/// the slot is recycled, so a stale id cannot address the node that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl NodeId {
    pub fn to_raw(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }

    pub fn from_raw(raw: u64) -> Self {
        Self {
            index: u32::try_from(raw & 0xffff_ffff).unwrap_or(0),
            generation: (raw >> 32) as u32,
        }
    }

    pub(crate) fn slot(self) -> usize {
        self.index as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
