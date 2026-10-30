//! Clamp, drag continuation, and the refresh threshold for one scroll.

use crate::ScrollPhase;
use crate::tree::{NodeId, Tree};

use super::window::{REFRESH_THRESHOLD, drag_origin, pull_delta};

/// Which distance arms a refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullMode {
    Off = 0,
    /// `max(0, -overscroll.y)` while dragging. Commit when the phase returns to idle.
    Overscroll = 1,
    /// Sum of top-edge clamp loss. Commit on drag end.
    Edge = 2,
}

crate::motion::coded_trinary!(PullMode, Off, Overscroll, Edge);

#[derive(Clone, Copy, Debug)]
pub struct DragStep {
    pub overflow_x: f32,
    pub overflow_y: f32,
    pub over_x: f32,
    pub over_y: f32,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingDrag {
    raw_x: f32,
    raw_y: f32,
    next_x: f32,
    next_y: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ParentLink {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Debug)]
pub struct ScrollController {
    viewport: Option<NodeId>,
    content: Option<NodeId>,
    content_min: f32,
    view_w: f32,
    view_h: f32,
    body_w: f32,
    body_h: f32,
    limits_x: f32,
    limits_y: f32,
    off_x: f32,
    off_y: f32,
    tracking: bool,
    drag_x: f32,
    drag_y: f32,
    rubber: bool,
    factor: f32,
    pull_mode: PullMode,
    threshold: f32,
    pull: f32,
    armed: bool,
    pending: Option<PendingDrag>,
    pub parent: Option<ParentLink>,
    axis: crate::ScrollAxis,
}

impl Default for ScrollController {
    fn default() -> Self {
        Self {
            viewport: None,
            content: None,
            content_min: 0.0,
            view_w: 0.0,
            view_h: 0.0,
            body_w: 0.0,
            body_h: 0.0,
            limits_x: 0.0,
            limits_y: 0.0,
            off_x: 0.0,
            off_y: 0.0,
            tracking: false,
            drag_x: 0.0,
            drag_y: 0.0,
            rubber: false,
            factor: 0.55,
            pull_mode: PullMode::Off,
            threshold: REFRESH_THRESHOLD,
            pull: 0.0,
            armed: false,
            pending: None,
            parent: None,
            axis: crate::ScrollAxis::Both,
        }
    }
}

impl ScrollController {
    pub fn bind(&mut self, viewport: Option<NodeId>, content: Option<NodeId>, tree: &Tree) {
        if let Some(id) = viewport {
            self.viewport = Some(id);
        }
        if let Some(id) = content {
            self.content = Some(id);
        }
        if let Some(id) = self.viewport
            && let Ok((x, y)) = tree.scroll_offset(id)
        {
            self.off_x = x;
            self.off_y = y;
        }
        self.refresh_boxes(tree);
    }

    pub fn configure(
        &mut self,
        content_min: f32,
        pull_mode: PullMode,
        threshold: f32,
        rubber: bool,
        factor: f32,
        parent: Option<ParentLink>,
    ) {
        if content_min.is_finite() && content_min >= 0.0 {
            self.content_min = content_min;
        }
        self.pull_mode = pull_mode;
        if threshold.is_finite() && threshold >= 0.0 {
            self.threshold = threshold;
        }
        self.rubber = rubber;
        if factor.is_finite() && factor >= 0.0 {
            self.factor = factor;
        }
        self.parent = parent;
        self.limits_y = (self.body_h - self.view_h)
            .max(0.0)
            .max(self.content_min - self.view_h)
            .max(0.0);
        self.apply_axis_limits();
    }

    pub fn limits(&self) -> (f32, f32) {
        (self.limits_x, self.limits_y)
    }

    pub fn refresh_boxes(&mut self, tree: &Tree) {
        let Some(viewport) = self.viewport else {
            return;
        };
        if let Ok(view) = tree.rect(viewport) {
            self.view_w = view.w;
            self.view_h = view.h;
        }
        if let Some(content) = self.content
            && let Ok(body) = tree.rect(content)
        {
            self.body_w = body.w;
            self.body_h = body.h;
        }
        if let Some(id) = self.viewport {
            self.axis = tree.scroll_axis(id).unwrap_or(crate::ScrollAxis::Both);
        }
        self.limits_x = (self.body_w - self.view_w).max(0.0);
        self.limits_y = (self.body_h - self.view_h)
            .max(0.0)
            .max(self.content_min - self.view_h)
            .max(0.0);
        self.apply_axis_limits();
    }

    /// A declared axis drops its cross component. `Both` with a zero limit still
    /// reports that component as overflow so a parent can take it.
    fn apply_axis_limits(&mut self) {
        let (x, y) = self.axis.filter(self.limits_x, self.limits_y);
        self.limits_x = x;
        self.limits_y = y;
    }

    fn clamp_pair(&self, x: f32, y: f32) -> (f32, f32) {
        if self.viewport.is_none() || self.content.is_none() {
            return (0.0, 0.0);
        }
        (x.clamp(0.0, self.limits_x), y.clamp(0.0, self.limits_y))
    }

    fn live(&self, tree: &Tree) -> (f32, f32) {
        self.viewport
            .and_then(|id| tree.scroll_offset(id).ok())
            .unwrap_or((self.off_x, self.off_y))
    }

    fn overscroll_of(&self, tree: &Tree) -> (f32, f32) {
        self.viewport
            .and_then(|id| tree.overscroll(id).ok())
            .unwrap_or((0.0, 0.0))
    }

    /// `true` when an overscroll refresh should commit.
    pub fn drag_by(&mut self, tree: &mut Tree, dx: f32, dy: f32) -> DragStep {
        self.refresh_boxes(tree);
        let (dx, dy) = self.axis.filter(dx, dy);
        let (live_x, live_y) = self.live(tree);
        if !self.tracking {
            self.tracking = true;
            let factor = if self.rubber { self.factor } else { 0.0 };
            let (sx, sy) = self.overscroll_of(tree);
            let origin = drag_origin(live_x, live_y, sx, sy, factor);
            self.drag_x = origin.0;
            self.drag_y = origin.1;
        }
        self.drag_x -= dx;
        self.drag_y -= dy;
        let raw_x = self.drag_x;
        let raw_y = self.drag_y;
        let (next_x, next_y) = self.clamp_pair(raw_x, raw_y);
        let overflow_x = raw_x - next_x;
        let overflow_y = raw_y - next_y;
        self.note_edge(live_y, raw_y, next_y);
        if overflow_x != 0.0 || overflow_y != 0.0 {
            self.pending = Some(PendingDrag {
                raw_x,
                raw_y,
                next_x,
                next_y,
            });
            let (over_x, over_y) = self.overscroll_of(tree);
            return DragStep {
                overflow_x,
                overflow_y,
                over_x,
                over_y,
                pending: true,
            };
        }
        self.pending = None;
        self.write_offset(tree, next_x, next_y, true);
        self.observe(tree, true);
        let (over_x, over_y) = self.overscroll_of(tree);
        DragStep {
            overflow_x: 0.0,
            overflow_y: 0.0,
            over_x,
            over_y,
            pending: false,
        }
    }

    /// `taken` means a parent or the host callback consumed the overflow.
    pub fn finish_drag(&mut self, tree: &mut Tree, taken: bool) -> bool {
        let Some(pending) = self.pending.take() else {
            return self.observe(tree, true);
        };
        let overflow = pending.raw_x != pending.next_x || pending.raw_y != pending.next_y;
        if !taken && self.rubber && overflow {
            if let Some(id) = self.viewport {
                let _ = tree.set_scroll_offset(id, pending.raw_x, pending.raw_y);
            }
            self.off_x = pending.next_x;
            self.off_y = pending.next_y;
        } else {
            self.write_offset(tree, pending.next_x, pending.next_y, true);
        }
        self.observe(tree, true)
    }

    /// Edge-mode refresh commits here. Overscroll mode waits for idle.
    pub fn drag_end(&mut self) -> bool {
        self.tracking = false;
        self.pending = None;
        if self.pull_mode != PullMode::Edge {
            return false;
        }
        let fire = self.pull > self.threshold;
        self.pull = 0.0;
        fire
    }

    pub fn wheel(&mut self, tree: &mut Tree, dx: f32, dy: f32) {
        self.refresh_boxes(tree);
        let (dx, dy) = self.axis.filter(dx, dy);
        let (x, y) = self.live(tree);
        self.write_offset(tree, x + dx, y + dy, true);
    }

    /// Keeps the cached offset on the tree and commits overscroll refresh at idle.
    pub fn tick(&mut self, tree: &mut Tree) -> bool {
        self.refresh_boxes(tree);
        if !self.tracking
            && let Some(id) = self.viewport
            && let Ok((x, y)) = tree.scroll_offset(id)
        {
            let (cx, cy) = self.clamp_pair(x, y);
            if (cx - x).abs() > f32::EPSILON || (cy - y).abs() > f32::EPSILON {
                self.write_offset(tree, cx, cy, true);
            } else {
                self.off_x = x;
                self.off_y = y;
            }
        }
        self.observe(tree, false)
    }

    /// Nested remainder. Returns whether this scroll moved.
    pub fn consume(&mut self, tree: &mut Tree, overflow_x: f32, overflow_y: f32) -> bool {
        self.refresh_boxes(tree);
        let (overflow_x, overflow_y) = self.axis.filter(overflow_x, overflow_y);
        if overflow_x == 0.0 && overflow_y == 0.0 {
            return false;
        }
        let Some(id) = self.viewport else {
            return false;
        };
        let (x, y) = self.live(tree);
        let can_x = overflow_x == 0.0
            || (overflow_x < 0.0 && x > 0.5)
            || (overflow_x > 0.0 && x < self.limits_x - 0.5);
        let can_y = overflow_y == 0.0
            || (overflow_y < 0.0 && y > 0.5)
            || (overflow_y > 0.0 && y < self.limits_y - 0.5);
        if !can_x && !can_y {
            return false;
        }
        let next_x = if can_x { x + overflow_x } else { x };
        let next_y = if can_y { y + overflow_y } else { y };
        let _ = tree.set_scroll_offset(id, next_x, next_y);
        self.off_x = if can_x { next_x } else { x };
        self.off_y = if can_y { next_y } else { y };
        let _ = id;
        true
    }

    pub fn query(&self, tree: &Tree, cmd: i32) -> (f32, f32) {
        match cmd {
            1 => self.limits(),
            2 => self.overscroll_of(tree),
            _ => self.live(tree),
        }
    }

    fn note_edge(&mut self, live_y: f32, raw_y: f32, next_y: f32) {
        if self.pull_mode != PullMode::Edge {
            return;
        }
        if self.body_h <= self.view_h {
            self.pull = 0.0;
            return;
        }
        let delta = pull_delta(live_y, raw_y, next_y);
        if delta > 0.0 {
            self.pull += delta;
        }
    }

    fn write_offset(&mut self, tree: &mut Tree, x: f32, y: f32, write: bool) {
        let (next_x, next_y) = self.clamp_pair(x, y);
        if (next_x - self.off_x).abs() <= f32::EPSILON
            && (next_y - self.off_y).abs() <= f32::EPSILON
        {
            return;
        }
        self.off_x = next_x;
        self.off_y = next_y;
        if write && let Some(id) = self.viewport {
            let _ = tree.set_scroll_offset(id, next_x, next_y);
        }
    }

    /// Finger-down samples arm. Idle without a finger commits.
    fn observe(&mut self, tree: &Tree, finger: bool) -> bool {
        if self.pull_mode != PullMode::Overscroll {
            return false;
        }
        let Some(id) = self.viewport else {
            return false;
        };
        let Ok(phase) = tree.scroll_phase(id) else {
            return false;
        };
        let (_, oy) = self.overscroll_of(tree);
        let distance = (-oy).max(0.0);
        let dragging = finger || phase == ScrollPhase::Dragging;
        if dragging && self.threshold > 0.0 && distance >= self.threshold {
            self.armed = true;
        }
        if !finger && phase == ScrollPhase::Idle && self.armed {
            self.armed = false;
            return true;
        }
        false
    }
}
