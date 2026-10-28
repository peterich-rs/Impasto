use std::collections::HashMap;
use std::ptr;

use super::key::ParaKey;
use crate::player::ffi;

pub(super) struct PaintSlot {
    pub(super) generation: u32,
    pub(super) refs: u32,
    pub(super) color: [u8; 4],
    pub(super) width: f32,
    pub(super) height: f32,
    /// `generation` when this slot was last measured or retained.
    pub(super) used: u64,
    pub(super) paragraph: ffi::ImpellerParagraph,
    pub(super) key: Option<ParaKey>,
}

impl PaintSlot {
    pub(super) fn empty() -> Self {
        Self {
            generation: 0,
            refs: 0,
            color: [0, 0, 0, 0],
            width: 0.0,
            height: 0.0,
            used: 0,
            paragraph: ptr::null_mut(),
            key: None,
        }
    }
}

pub(crate) struct PaintStore {
    pub(super) slots: Vec<PaintSlot>,
    pub(super) index: HashMap<u64, Vec<u32>>,
    pub(super) free: Vec<u32>,
}

impl PaintStore {
    pub(super) fn new() -> Self {
        Self {
            slots: Vec::new(),
            index: HashMap::new(),
            free: Vec::new(),
        }
    }
}
