//! Pointer configuration and fling velocity. Claim policy lives on `Tree::pointer`.

use std::collections::HashMap;

use crate::{
    geometry::NodeId,
    input::{PointerId, PointerKind, PointerSample},
};

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
            slop: DOUBLE_TAP_SLOP,
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

/// Default distance between the taps in a double tap.
pub const DOUBLE_TAP_SLOP: f32 = 8.0;

pub(crate) fn drag_slop(kind: PointerKind) -> f32 {
    match kind {
        PointerKind::Touch => 12.0,
        PointerKind::Mouse | PointerKind::Trackpad | PointerKind::Pen => 4.0,
    }
}

/// How long a still finger must stay inside the slop before a long press wins.
pub const LONG_PRESS_MS: f64 = 500.0;

const VELOCITY_SAMPLES: usize = 16;
const RELEASE_NOISE_MS: f64 = 2.0;
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

/// Recent positions for a weighted linear fit, in scene points per millisecond.
#[derive(Clone)]
pub struct VelocityRing {
    samples: [PointerSample; VELOCITY_SAMPLES],
    next: usize,
    len: usize,
}

impl VelocityRing {
    pub fn new() -> Self {
        Self {
            samples: [PointerSample::default(); VELOCITY_SAMPLES],
            next: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, x: f32, y: f32, time_ms: f64) {
        if !x.is_finite() || !y.is_finite() || !time_ms.is_finite() {
            return;
        }
        let sample = PointerSample { x, y, time_ms };
        if self.len > 0 {
            let last = (self.next + VELOCITY_SAMPLES - 1) % VELOCITY_SAMPLES;
            if time_ms < self.samples[last].time_ms {
                return;
            }
            if time_ms == self.samples[last].time_ms {
                self.samples[last] = sample;
                return;
            }
        }
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % VELOCITY_SAMPLES;
        self.len = (self.len + 1).min(VELOCITY_SAMPLES);
    }

    /// Estimate speed at release time. Prefer the fit before the final 2 ms
    /// of contact if that interval already provides enough samples.
    pub fn velocity_at(&self, now_ms: f64) -> (f32, f32) {
        self.fit(now_ms, RELEASE_NOISE_MS)
            .or_else(|| self.fit(now_ms, 0.0))
            .unwrap_or((0.0, 0.0))
    }

    fn fit(&self, now_ms: f64, release_noise: f64) -> Option<(f32, f32)> {
        let first = (self.next + VELOCITY_SAMPLES - self.len) % VELOCITY_SAMPLES;
        let mut weight = 0.0;
        let mut mean = [0.0; 3];
        let mut covariance = [0.0; 3];
        let mut span = None;
        for index in 0..self.len {
            let sample = self.samples[(first + index) % VELOCITY_SAMPLES];
            let age = now_ms - sample.time_ms;
            if !(release_noise..=VELOCITY_WINDOW_MS).contains(&age) {
                continue;
            }
            let w = 2.0 - age / VELOCITY_WINDOW_MS;
            let point = [-age, f64::from(sample.x), f64::from(sample.y)];
            let delta = std::array::from_fn::<_, 3, _>(|axis| point[axis] - mean[axis]);
            weight += w;
            for axis in 0..3 {
                mean[axis] += w * delta[axis] / weight;
                covariance[axis] += w * delta[0] * (point[axis] - mean[axis]);
            }
            let start = span.get_or_insert((age, age));
            start.1 = age;
        }
        let (oldest, newest) = span?;
        if oldest - newest < RELEASE_NOISE_MS || covariance[0] <= 0.0 {
            return None;
        }
        Some((
            crate::cast::f32_from_f64(covariance[1] / covariance[0]),
            crate::cast::f32_from_f64(covariance[2] / covariance[0]),
        ))
    }
}

impl Default for VelocityRing {
    fn default() -> Self {
        Self::new()
    }
}
