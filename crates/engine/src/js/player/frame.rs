//! Per-frame input counters.

use std::cell::Cell;
use std::time::Duration;

pub(super) struct FrameMarks {
    pub(super) pointers: Cell<u32>,
    pub(super) keys: Cell<u32>,
    pub(super) edits: Cell<u32>,
    pub(super) ime: Cell<u32>,
    pub(super) scrolls: Cell<u32>,
    pub(super) focus: Cell<u32>,
    pub(super) blur: Cell<u32>,
    pub(super) gestures: Cell<u32>,
    /// Scene setter calls since the last reset.
    pub(super) sets: Cell<u32>,
    pub(super) trace_enabled: Cell<bool>,
    pub(super) callback_active: Cell<bool>,
    pub(super) callbacks: Cell<u32>,
    pub(super) callback_time: Cell<Duration>,
    pub(super) mutation_calls: Cell<u32>,
    pub(super) mutation_equal: Cell<u32>,
    pub(super) text_writes: Cell<u32>,
    pub(super) mutation_time: Cell<Duration>,
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
        self.sets.set(0);
        self.reset_trace();
    }

    pub(super) fn reset_trace(&self) {
        self.callbacks.set(0);
        self.callback_time.set(Duration::ZERO);
        self.mutation_calls.set(0);
        self.mutation_equal.set(0);
        self.text_writes.set(0);
        self.mutation_time.set(Duration::ZERO);
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
            sets: Cell::new(0),
            trace_enabled: Cell::new(false),
            callback_active: Cell::new(false),
            callbacks: Cell::new(0),
            callback_time: Cell::new(Duration::ZERO),
            mutation_calls: Cell::new(0),
            mutation_equal: Cell::new(0),
            text_writes: Cell::new(0),
            mutation_time: Cell::new(Duration::ZERO),
        }
    }
}

pub(super) fn bump(cell: &Cell<u32>) {
    cell.set(cell.get().saturating_add(1));
}

pub(super) fn note_gesture(marks: &FrameMarks, gesture: &crate::tree::Gesture) {
    bump(&marks.gestures);
    match gesture {
        crate::tree::Gesture::Scroll(_, _, _) | crate::tree::Gesture::ScrollPhaseChanged { .. } => {
            bump(&marks.scrolls);
        }
        crate::tree::Gesture::Focus(_) => bump(&marks.focus),
        crate::tree::Gesture::Blur(_) => bump(&marks.blur),
        _ => {}
    }
}

pub(super) fn elapsed_ms(start: std::time::Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
