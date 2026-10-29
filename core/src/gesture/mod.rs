//! Pointer contest. Recognizers are a trait, so a new gesture is another type.

use crate::{NodeId, Phase, PointerEvent, PointerId};

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
pub enum Vote {
    Undecided,
    Accept,
    Reject,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecognizerClaim {
    Tap,
    LongPress,
    Drag { axis: DragAxis },
    Scroll,
    TextSelect { axis: DragAxis },
}

/// Window and slop for a second tap. The wait lives on the arena, not here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleTapConfig {
    pub slop: f32,
    pub window_ms: f64,
}

impl Default for DoubleTapConfig {
    fn default() -> Self {
        Self {
            slop: SLOP,
            window_ms: 300.0,
        }
    }
}

/// Which optional recognizers a node contributes. Scroll and text selection
/// are registered by the node kind, not by this set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecognizerSet {
    pub tap: bool,
    pub long_press: bool,
    pub drag: Option<DragAxis>,
    pub double_tap: Option<DoubleTapConfig>,
}

pub trait Recognizer {
    fn claim(&self) -> RecognizerClaim;
    fn on_pointer(&mut self, event: &PointerEvent, path: &HitPath) -> Vote;
    fn on_accept(&mut self) {}
    fn on_reject(&mut self) {}
}

/// Distance a move must exceed before an axis can win. Same length as `DRAG_SLOP`.
pub const SLOP: f32 = 8.0;

pub fn past_slop(origin_x: f32, origin_y: f32, x: f32, y: f32) -> bool {
    let dx = x - origin_x;
    let dy = y - origin_y;
    dx * dx + dy * dy > SLOP * SLOP
}

const LONG_PRESS_MS: f64 = 500.0;
const VELOCITY_SAMPLES: usize = 8;
/// Samples older than this window do not shape the fling velocity.
pub const VELOCITY_WINDOW_MS: f64 = 120.0;

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
        ((dx / dt) as f32, (dy / dt) as f32)
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

pub struct TapRecognizer {
    pub target: NodeId,
    origin_x: f32,
    origin_y: f32,
    inside: bool,
}

impl TapRecognizer {
    pub fn new(target: NodeId, x: f32, y: f32) -> Self {
        Self {
            target,
            origin_x: x,
            origin_y: y,
            inside: true,
        }
    }
}

impl Recognizer for TapRecognizer {
    fn claim(&self) -> RecognizerClaim {
        RecognizerClaim::Tap
    }

    fn on_pointer(&mut self, event: &PointerEvent, path: &HitPath) -> Vote {
        match event.phase {
            Phase::Move => {
                self.inside = path.contains(self.target);
                if past_slop(self.origin_x, self.origin_y, event.x, event.y) {
                    Vote::Reject
                } else {
                    Vote::Undecided
                }
            }
            Phase::Up => {
                if self.inside && !past_slop(self.origin_x, self.origin_y, event.x, event.y) {
                    Vote::Accept
                } else {
                    Vote::Reject
                }
            }
            Phase::Cancel => Vote::Reject,
            Phase::Down | Phase::Wheel | Phase::Zoom => Vote::Undecided,
        }
    }
}

pub struct LongPressRecognizer {
    pub target: NodeId,
    origin_x: f32,
    origin_y: f32,
    down_ms: f64,
    fired: bool,
}

impl LongPressRecognizer {
    pub fn new(target: NodeId, x: f32, y: f32, time_ms: f64) -> Self {
        Self {
            target,
            origin_x: x,
            origin_y: y,
            down_ms: time_ms,
            fired: false,
        }
    }
}

impl Recognizer for LongPressRecognizer {
    fn claim(&self) -> RecognizerClaim {
        RecognizerClaim::LongPress
    }

    fn on_pointer(&mut self, event: &PointerEvent, _path: &HitPath) -> Vote {
        if self.fired {
            return Vote::Accept;
        }
        if past_slop(self.origin_x, self.origin_y, event.x, event.y) {
            return Vote::Reject;
        }
        match event.phase {
            Phase::Move | Phase::Up
                if event.time_ms.is_finite() && event.time_ms - self.down_ms >= LONG_PRESS_MS =>
            {
                self.fired = true;
                Vote::Accept
            }
            Phase::Cancel => Vote::Reject,
            _ => Vote::Undecided,
        }
    }
}

/// Shared axial drag: past slop, then vote by axis. Drag and text-select differ only in claim.
struct AxisDragState {
    axis: DragAxis,
    origin_x: f32,
    origin_y: f32,
}

impl AxisDragState {
    fn new(axis: DragAxis, x: f32, y: f32) -> Self {
        Self {
            axis,
            origin_x: x,
            origin_y: y,
        }
    }

    fn vote(&self, event: &PointerEvent) -> Vote {
        if !past_slop(self.origin_x, self.origin_y, event.x, event.y) {
            return Vote::Undecided;
        }
        if self
            .axis
            .matches(event.x - self.origin_x, event.y - self.origin_y)
        {
            Vote::Accept
        } else {
            Vote::Reject
        }
    }
}

pub struct DragRecognizer(AxisDragState);

impl DragRecognizer {
    pub fn new(_target: NodeId, axis: DragAxis, x: f32, y: f32) -> Self {
        Self(AxisDragState::new(axis, x, y))
    }
}

impl Recognizer for DragRecognizer {
    fn claim(&self) -> RecognizerClaim {
        RecognizerClaim::Drag { axis: self.0.axis }
    }

    fn on_pointer(&mut self, event: &PointerEvent, _path: &HitPath) -> Vote {
        self.0.vote(event)
    }
}

/// One finger owns a scroll. A second pointer does not join the lock.
pub struct ScrollRecognizer {
    pub scroll: NodeId,
    pointer: PointerId,
    origin_x: f32,
    origin_y: f32,
    held_by_other: bool,
}

impl ScrollRecognizer {
    pub fn new(scroll: NodeId, pointer: PointerId, x: f32, y: f32, held_by_other: bool) -> Self {
        Self {
            scroll,
            pointer,
            origin_x: x,
            origin_y: y,
            held_by_other,
        }
    }

    /// `locks` maps a scroll to the pointer that already won it.
    pub fn locked_by_other(
        locks: &std::collections::HashMap<NodeId, PointerId>,
        scroll: NodeId,
        pointer: Option<PointerId>,
    ) -> bool {
        locks
            .get(&scroll)
            .is_some_and(|owner| pointer.is_none_or(|pointer| pointer != *owner))
    }
}

impl Recognizer for ScrollRecognizer {
    fn claim(&self) -> RecognizerClaim {
        RecognizerClaim::Scroll
    }

    fn on_pointer(&mut self, event: &PointerEvent, _path: &HitPath) -> Vote {
        if self.held_by_other || event.pointer != self.pointer {
            return Vote::Reject;
        }
        if past_slop(self.origin_x, self.origin_y, event.x, event.y) {
            Vote::Accept
        } else {
            Vote::Undecided
        }
    }
}

pub struct TextSelectRecognizer(AxisDragState);

impl TextSelectRecognizer {
    pub fn new(_target: NodeId, axis: DragAxis, x: f32, y: f32) -> Self {
        Self(AxisDragState::new(axis, x, y))
    }
}

impl Recognizer for TextSelectRecognizer {
    fn claim(&self) -> RecognizerClaim {
        RecognizerClaim::TextSelect { axis: self.0.axis }
    }

    fn on_pointer(&mut self, event: &PointerEvent, _path: &HitPath) -> Vote {
        self.0.vote(event)
    }
}

/// Members collected for one pointer. The winner is the first `Accept`.
pub struct PointerContest {
    members: Vec<Box<dyn Recognizer>>,
    winner: Option<usize>,
}

impl PointerContest {
    pub fn new(members: Vec<Box<dyn Recognizer>>) -> Self {
        Self {
            members,
            winner: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    #[allow(dead_code)]
    pub fn winner(&self) -> Option<&dyn Recognizer> {
        self.winner.map(|index| self.members[index].as_ref())
    }

    /// Feeds `event`. An `Accept` settles the contest and rejects the rest.
    pub fn on_pointer(&mut self, event: &PointerEvent, path: &HitPath) -> Option<RecognizerClaim> {
        if let Some(index) = self.winner {
            let vote = self.members[index].on_pointer(event, path);
            if vote == Vote::Reject {
                self.members[index].on_reject();
                self.winner = None;
            }
            return self.members.get(index).map(|member| member.claim());
        }
        let mut accepted: Option<usize> = None;
        for (index, member) in self.members.iter_mut().enumerate() {
            match member.on_pointer(event, path) {
                Vote::Accept if accepted.is_none() => accepted = Some(index),
                Vote::Reject => member.on_reject(),
                Vote::Accept | Vote::Undecided => {}
            }
        }
        let Some(index) = accepted else {
            return None;
        };
        for (other, member) in self.members.iter_mut().enumerate() {
            if other != index {
                member.on_reject();
            } else {
                member.on_accept();
            }
        }
        self.winner = Some(index);
        Some(self.members[index].claim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Button, Buttons, Modifiers, PointerKind};

    fn event(phase: Phase, x: f32, y: f32, time_ms: f64) -> PointerEvent {
        PointerEvent {
            pointer: PointerId(1),
            kind: PointerKind::Touch,
            phase,
            x,
            y,
            time_ms,
            button: Button::Primary,
            buttons: Buttons::from_button(Button::Primary),
            modifiers: Modifiers::empty(),
            dx: 0.0,
            dy: 0.0,
            zoom: 0.0,
        }
    }

    #[test]
    fn a_horizontal_drag_rejects_a_vertical_move() {
        let mut drag = DragRecognizer::new(NodeId::from_raw(1), DragAxis::Horizontal, 0.0, 0.0);
        let path = HitPath::default();
        assert_eq!(
            drag.on_pointer(&event(Phase::Move, 0.0, 40.0, 20.0), &path),
            Vote::Reject
        );
        let mut drag = DragRecognizer::new(NodeId::from_raw(1), DragAxis::Horizontal, 0.0, 0.0);
        assert_eq!(
            drag.on_pointer(&event(Phase::Move, 40.0, 0.0, 20.0), &path),
            Vote::Accept
        );
    }

    #[test]
    fn a_second_finger_does_not_join_a_locked_scroll() {
        let mut locks = std::collections::HashMap::new();
        let scroll = NodeId::from_raw(7);
        locks.insert(scroll, PointerId(1));
        assert!(ScrollRecognizer::locked_by_other(
            &locks,
            scroll,
            Some(PointerId(2))
        ));
        assert!(!ScrollRecognizer::locked_by_other(
            &locks,
            scroll,
            Some(PointerId(1))
        ));
        let mut recognizer = ScrollRecognizer::new(scroll, PointerId(2), 0.0, 0.0, true);
        let path = HitPath::default();
        assert_eq!(
            recognizer.on_pointer(&event(Phase::Move, 0.0, 40.0, 16.0), &path),
            Vote::Reject
        );
    }

    #[test]
    fn a_long_press_accepts_after_half_a_second() {
        let mut press = LongPressRecognizer::new(NodeId::from_raw(3), 1.0, 1.0, 0.0);
        let path = HitPath::default();
        assert_eq!(
            press.on_pointer(&event(Phase::Move, 1.0, 1.0, 400.0), &path),
            Vote::Undecided
        );
        assert_eq!(
            press.on_pointer(&event(Phase::Move, 1.0, 1.0, 500.0), &path),
            Vote::Accept
        );
        let mut contest = PointerContest::new(vec![Box::new(press)]);
        assert!(
            contest
                .on_pointer(&event(Phase::Move, 1.0, 1.0, 500.0), &path)
                .is_some()
        );
        assert!(contest.winner().is_some());
    }
}
