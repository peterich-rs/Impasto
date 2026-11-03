//! Pointer configuration and fling velocity. Claim policy lives on `Tree::pointer`.

use std::collections::HashMap;

use crate::{NodeId, PointerId};

/// One node the hit walk entered, root first. `x` and `y` are local to that node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitEntry {
    pub id: NodeId,
    pub x: f32,
    pub y: f32,
}

/// Reused hit buffer. Cleared by the caller before each walk.
#[derive(Clone, Debug, Default)]
pub struct HitPath {
    entries: Vec<HitEntry>,
}

impl HitPath {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push(&mut self, entry: HitEntry) {
        self.entries.push(entry);
    }

    pub fn pop(&mut self) {
        self.entries.pop();
    }

    pub fn entries(&self) -> &[HitEntry] {
        &self.entries
    }

    pub fn leaf(&self) -> Option<NodeId> {
        self.entries.last().map(|entry| entry.id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragAxis {
    Horizontal,
    Vertical,
    Free,
}

impl DragAxis {
    pub fn matches(self, dx: f32, dy: f32) -> bool {
        match self {
            DragAxis::Free => dx != 0.0 || dy != 0.0,
            DragAxis::Horizontal => dx.abs() >= dy.abs() && dx != 0.0,
            DragAxis::Vertical => dy.abs() > dx.abs() && dy != 0.0,
        }
    }
}

/// Window and slop for a second tap. The wait lives on the route table, not here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleTapConfig {
    pub slop: f32,
    pub window_ms: f64,
}

impl Default for DoubleTapConfig {
    fn default() -> Self {
        Self {
            slop: DRAG_SLOP,
            window_ms: 300.0,
        }
    }
}

/// Which optional gestures a node contributes. Scroll and text selection
/// come from the node kind, not from this set. This is configuration, not a contest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecognizerSet {
    pub tap: bool,
    pub long_press: bool,
    pub drag: Option<DragAxis>,
    pub double_tap: Option<DoubleTapConfig>,
}

/// Distance a move must exceed before a drag can lock. Not Flutter's current `kTouchSlop`.
pub const DRAG_SLOP: f32 = 8.0;

/// How long a still finger must stay inside the slop before a long press wins.
pub const LONG_PRESS_MS: f64 = 500.0;

const VELOCITY_SAMPLES: usize = 8;
/// Samples older than this window do not shape the fling velocity.
pub const VELOCITY_WINDOW_MS: f64 = 120.0;

/// `locks` maps a scroll to the pointer that already won it.
pub fn scroll_locked_by_other(
    locks: &HashMap<NodeId, PointerId>,
    scroll: NodeId,
    pointer: Option<PointerId>,
) -> bool {
    locks
        .get(&scroll)
        .is_some_and(|owner| pointer.is_none_or(|pointer| pointer != *owner))
}

#[derive(Clone, Copy)]
struct Sample {
    x: f32,
    y: f32,
    time_ms: f64,
}

/// Time-weighted velocity over at most eight samples, in scene points per millisecond.
#[derive(Clone)]
pub struct VelocityRing {
    samples: Vec<Sample>,
}

impl VelocityRing {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(VELOCITY_SAMPLES),
        }
    }

    pub fn push(&mut self, x: f32, y: f32, time_ms: f64) {
        if self.samples.len() == VELOCITY_SAMPLES {
            self.samples.remove(0);
        }
        self.samples.push(Sample { x, y, time_ms });
    }

    /// Velocity over the samples inside `WINDOW_MS` of `now_ms`. Samples older
    /// than the window do not count, so a flick followed by a stall reports
    /// the stall. This is the weighting iOS gesture recognizers use.
    pub fn velocity_at(&self, now_ms: f64) -> (f32, f32) {
        let fresh: Vec<Sample> = self
            .samples
            .iter()
            .copied()
            .filter(|sample| now_ms - sample.time_ms <= VELOCITY_WINDOW_MS)
            .collect();
        let mut dx = 0.0f64;
        let mut dy = 0.0f64;
        let mut dt = 0.0f64;
        for pair in fresh.windows(2) {
            let step = pair[1].time_ms - pair[0].time_ms;
            if step.is_finite() && step > 0.0 {
                dx += f64::from(pair[1].x - pair[0].x);
                dy += f64::from(pair[1].y - pair[0].y);
                dt += step;
            }
        }
        if dt <= 0.0 {
            return (0.0, 0.0);
        }
        (
            crate::cast::f32_from_f64(dx / dt),
            crate::cast::f32_from_f64(dy / dt),
        )
    }

    pub fn velocity(&self) -> (f32, f32) {
        match self.samples.last() {
            Some(latest) => self.velocity_at(latest.time_ms),
            None => (0.0, 0.0),
        }
    }
}

impl Default for VelocityRing {
    fn default() -> Self {
        Self::new()
    }
}
