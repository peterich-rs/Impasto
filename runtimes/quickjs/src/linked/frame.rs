//! Per-frame input counters.

use std::cell::Cell;

pub(super) struct FrameMarks {
    pub(super) pointers: Cell<u32>,
    pub(super) keys: Cell<u32>,
    pub(super) edits: Cell<u32>,
    pub(super) ime: Cell<u32>,
    pub(super) scrolls: Cell<u32>,
    pub(super) focus: Cell<u32>,
    pub(super) blur: Cell<u32>,
    pub(super) gestures: Cell<u32>,
}

impl FrameMarks {
    pub(super) fn reset(&self) {
        self.pointers.set(0);
        self.keys.set(0);
        self.edits.set(0);
        self.ime.set(0);
        self.scrolls.set(0);
        self.focus.set(0);
        self.blur.set(0);
        self.gestures.set(0);
    }
}

impl Default for FrameMarks {
    fn default() -> Self {
        Self {
            pointers: Cell::new(0),
            keys: Cell::new(0),
            edits: Cell::new(0),
            ime: Cell::new(0),
            scrolls: Cell::new(0),
            focus: Cell::new(0),
            blur: Cell::new(0),
            gestures: Cell::new(0),
        }
    }
}

pub(super) fn bump(cell: &Cell<u32>) {
    cell.set(cell.get().saturating_add(1));
}

pub(super) fn note_gesture(marks: &FrameMarks, gesture: &uicore::Gesture) {
    bump(&marks.gestures);
    match gesture {
        uicore::Gesture::Scroll(_, _, _) | uicore::Gesture::ScrollPhaseChanged { .. } => {
            bump(&marks.scrolls);
        }
        uicore::Gesture::Focus(_) => bump(&marks.focus),
        uicore::Gesture::Blur(_) => bump(&marks.blur),
        _ => {}
    }
}

pub(super) fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
