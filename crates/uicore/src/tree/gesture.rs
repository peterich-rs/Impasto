//! Press, drag, focus, hover, wheel, and the right button.
//!
//! The right button keeps its own press. It never writes [`Tree::pressed`] or [`Tree::drag`].

use std::collections::HashMap;

use super::node::Kind;
use super::{FocusDirection, Gesture, NodeId, Tree};
use crate::{Button, Error, Phase, PointerEvent, PointerId};

/// Routes captured at pointer down. A later structure change does not drop them
/// unless the target node itself is removed.
pub(super) struct GestureArena {
    pub(super) routes: HashMap<PointerId, PointerRoute>,
    /// Scroll nodes that already have a locked pointer. A second finger does not join.
    pub(super) scroll_locks: HashMap<NodeId, PointerId>,
    pub(super) hovered: Option<NodeId>,
    /// Leave gestures produced by a structure change, delivered on the next pointer or remove.
    pub(super) pending: Vec<Gesture>,
    /// First tap of a double-tap pair. Survives the route being dropped on up.
    pub(super) pending_tap: Option<PendingTap>,
    /// Scene point of the pointer event currently being dispatched.
    pub(super) pointer_at: (f32, f32),
}

impl Default for GestureArena {
    fn default() -> Self {
        Self {
            routes: HashMap::new(),
            scroll_locks: HashMap::new(),
            hovered: None,
            pending: Vec::new(),
            pending_tap: None,
            pointer_at: (0.0, 0.0),
        }
    }
}

pub(super) struct PendingTap {
    node: NodeId,
    x: f32,
    y: f32,
    time_ms: f64,
    /// `Some` schedules [`Gesture::Click`] when [`Tree::tick`] passes this time.
    deadline_ms: Option<f64>,
    focus: Option<NodeId>,
    window_ms: f64,
    slop: f32,
}

impl GestureArena {
    pub(super) fn clear_index(&mut self, index: u32) {
        self.routes.retain(|_, route| {
            if route.pressed.is_some_and(|pressed| pressed.index == index) {
                route.pressed = None;
            }
            if route
                .secondary
                .as_ref()
                .is_some_and(|press| press.target.index == index)
            {
                route.secondary = None;
            }
            if route
                .drag
                .as_ref()
                .is_some_and(|drag| drag.scroll.is_some_and(|scroll| scroll.index == index))
            {
                route.drag = None;
            }
            !route.is_empty()
        });
        self.scroll_locks.retain(|node, _| node.index != index);
    }
}

/// Distance from the pointer-down point that a move must exceed before a scroll
/// wins the gesture. The same length as Flutter's `kTouchSlop`.
const DRAG_SLOP: f32 = 8.0;

#[derive(Clone, Copy)]
pub(super) struct Sample {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) time_ms: f64,
}

pub(super) struct DragState {
    /// Deepest node under the pointer at press time.
    pub(super) hit: Option<NodeId>,
    /// Scroll or selection target after the gesture locks.
    pub(super) scroll: Option<NodeId>,
    pub(super) locked: bool,
    /// The long-press recognizer already won. Lift releases without a click.
    pub(super) long_press: bool,
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) last_x: f32,
    pub(super) last_y: f32,
    pub(super) previous_move: Option<Sample>,
    pub(super) latest_move: Option<Sample>,
    pub(super) velocity: crate::VelocityRing,
}

/// Press, drag, and right button for one [`crate::PointerId`].
pub(super) struct PointerRoute {
    pub(super) pressed: Option<NodeId>,
    pub(super) secondary: Option<SecondaryPress>,
    pub(super) drag: Option<DragState>,
    pub(super) contest: Option<crate::gesture::PointerContest>,
    /// The down that produced [`Gesture::DoubleTap`] must not also click.
    pub(super) consumed_double: bool,
}

impl PointerRoute {
    pub(super) fn is_empty(&self) -> bool {
        self.pressed.is_none()
            && self.secondary.is_none()
            && self.drag.is_none()
            && self
                .contest
                .as_ref()
                .is_none_or(|contest| contest.is_empty())
            && !self.consumed_double
    }
}

impl Default for PointerRoute {
    fn default() -> Self {
        Self {
            pressed: None,
            secondary: None,
            drag: None,
            contest: None,
            consumed_double: false,
        }
    }
}

/// Right button press. Independent of the primary press on the same route.
pub(super) struct SecondaryPress {
    pub(super) target: NodeId,
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) cancelled: bool,
}

impl Tree {
    /// Marks `id` as a tap-to-focus target. Any node may carry the flag.
    /// Clearing it on the focused node blurs.
    pub fn set_focusable(&mut self, id: NodeId, focusable: bool) -> Result<Vec<Gesture>, Error> {
        self.node_mut(id)?.set_focusable_flag(focusable);
        if !focusable && self.focused == Some(id) {
            Ok(self.move_focus(None))
        } else {
            Ok(Vec::new())
        }
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Whether `id` is marked focusable. Unknown ids are [`Error::UnknownNode`].
    pub fn is_focusable(&self, id: NodeId) -> Result<bool, Error> {
        Ok(self.node(id)?.is_focusable())
    }

    /// Moves focus to `id` when it is marked focusable. The returned transition
    /// is empty when `id` is already focused.
    pub fn focus(&mut self, id: NodeId) -> Result<Vec<Gesture>, Error> {
        if !self.node(id)?.is_focusable() {
            return Err(Error::NotFocusable);
        }
        if !self.inside_scope(id) {
            return Ok(Vec::new());
        }
        Ok(self.move_focus(Some(id)))
    }

    pub fn blur(&mut self) -> Vec<Gesture> {
        self.move_focus(None)
    }

    /// Hit-tests the previous layout and updates the press, drag, and focus gestures.
    ///
    /// A hit on the way down is [`Gesture::Press`]. A move that stays within
    /// [`DRAG_SLOP`] keeps that press. Past the slop, a region that
    /// [`Tree::set_drag_claim`]s the dominant axis locks as a selection drag.
    /// Otherwise the scroll chain does: a scroll at its edge yields to an
    /// ancestor that can still move on that axis. One scroll keeps one locked
    /// pointer. A lift inside the pressed region, with no lock, is
    /// [`Gesture::Release`] then [`Gesture::Click`], and that click is what moves
    /// focus. A locked drag does not. Does not lay out.
    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Gesture> {
        self.arena.pointer_at = (event.x, event.y);
        let mut gestures = self.drain_pending();
        if !self.laid_out {
            return gestures;
        }
        let mut route = self.arena.routes.remove(&event.pointer).unwrap_or_default();
        let produced = match event.phase {
            Phase::Wheel => self.pointer_wheel(event),
            Phase::Zoom => self.pointer_zoom(event),
            Phase::Cancel => self.pointer_cancel(&mut route, event.pointer),
            Phase::Down | Phase::Move | Phase::Up if event.button == Button::Secondary => {
                self.pointer_secondary(&mut route, event)
            }
            Phase::Down | Phase::Up if event.button != Button::Primary => Vec::new(),
            Phase::Move if event.button != Button::None && event.button != Button::Primary => {
                Vec::new()
            }
            Phase::Down => self.pointer_down(&mut route, event),
            Phase::Move => self.pointer_move(&mut route, event),
            Phase::Up => self.pointer_up(&mut route, event),
        };
        if !route.is_empty() {
            self.arena.routes.insert(event.pointer, route);
        }
        gestures.extend(produced);
        gestures
    }

    /// Primary press only. Callers keep other buttons out of this path.
    pub(super) fn pointer_down(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        route.drag = None;
        route.pressed = None;
        route.consumed_double = false;
        let mut early = self.take_second_tap(&event);
        let hit = self.hit_root(event.x, event.y);
        let pressed = hit.and_then(|id| self.region_of(id));
        let scroll = hit.and_then(|id| self.drag_of(id));
        route.pressed = pressed;
        if pressed.is_some() || scroll.is_some() {
            route.drag = Some(DragState {
                hit,
                scroll,
                locked: false,
                long_press: false,
                origin_x: event.x,
                origin_y: event.y,
                last_x: event.x,
                last_y: event.y,
                previous_move: None,
                latest_move: None,
                velocity: crate::VelocityRing::new(),
            });
        }
        route.contest = Some(self.contest_for(&event));
        if early
            .iter()
            .any(|gesture| matches!(gesture, Gesture::DoubleTap(_)))
        {
            route.consumed_double = true;
        }
        early.extend(pressed.into_iter().map(Gesture::Press));
        early
    }

    /// Second down inside the window and slop. The first up did not accept tap.
    fn take_second_tap(&mut self, event: &PointerEvent) -> Vec<Gesture> {
        let Some(pending) = self.arena.pending_tap.take() else {
            return Vec::new();
        };
        let hit = self.hit_root(event.x, event.y);
        let target = hit
            .and_then(|id| self.region_of(id))
            .unwrap_or(pending.node);
        let inside = event.time_ms >= pending.time_ms
            && event.time_ms - pending.time_ms <= pending.window_ms
            && !beyond(pending.x, pending.y, event.x, event.y, pending.slop)
            && target == pending.node;
        if inside {
            return vec![Gesture::DoubleTap(pending.node)];
        }
        Vec::new()
    }

    /// Click scheduled by a tap that was waiting for a double tap.
    pub fn tick(&mut self, now_ms: f64) -> Vec<Gesture> {
        let Some(pending) = self.arena.pending_tap.take() else {
            return Vec::new();
        };
        if pending
            .deadline_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            let mut gestures = vec![Gesture::Click(pending.node)];
            gestures.extend(self.move_focus(pending.focus));
            return gestures;
        }
        if pending.deadline_ms.is_none() && now_ms - pending.time_ms > pending.window_ms {
            return Vec::new();
        }
        self.arena.pending_tap = Some(pending);
        Vec::new()
    }

    fn pointer_zoom(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(hit) = self.hit_root(event.x, event.y) else {
            return Vec::new();
        };
        let Some(id) = self.nearest_transform(hit) else {
            return Vec::new();
        };
        vec![Gesture::Zoom {
            id,
            scale: event.zoom,
        }]
    }

    fn nearest_transform(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            if self
                .get(id)
                .is_some_and(|node| node.kind == Kind::Transform)
            {
                return Some(id);
            }
            id = self.get(id)?.parent?;
        }
    }

    fn contest_for(&self, event: &PointerEvent) -> crate::gesture::PointerContest {
        use crate::{
            DragRecognizer, LongPressRecognizer, Recognizer, ScrollRecognizer, TapRecognizer,
            TextSelectRecognizer,
        };
        let mut path = crate::HitPath::default();
        self.hit_path(event.x, event.y, &mut path);
        let mut members: Vec<Box<dyn Recognizer>> = Vec::new();
        for entry in path.entries() {
            let Some(node) = self.get(entry.id) else {
                continue;
            };
            if node.kind == Kind::Scroll {
                let held = ScrollRecognizer::locked_by_other(
                    &self.arena.scroll_locks,
                    entry.id,
                    Some(event.pointer),
                );
                members.push(Box::new(ScrollRecognizer::new(
                    entry.id,
                    event.pointer,
                    event.x,
                    event.y,
                    held,
                )));
            }
            if let Some(axis) = text_select_axis(node) {
                members.push(Box::new(TextSelectRecognizer::new(
                    entry.id, axis, event.x, event.y,
                )));
            }
            if node.recognizers.tap || node.hit_role() == super::node::Hit::Region {
                members.push(Box::new(TapRecognizer::new(entry.id, event.x, event.y)));
            }
            if node.recognizers.long_press {
                members.push(Box::new(LongPressRecognizer::new(
                    entry.id,
                    event.x,
                    event.y,
                    event.time_ms,
                )));
            }
            if let Some(axis) = node.recognizers.drag {
                members.push(Box::new(DragRecognizer::new(
                    entry.id, axis, event.x, event.y,
                )));
            }
        }
        crate::gesture::PointerContest::new(members)
    }

    pub fn set_recognizers(&mut self, id: NodeId, set: crate::RecognizerSet) -> Result<(), Error> {
        self.node_mut(id)?.recognizers = set;
        Ok(())
    }

    pub(super) fn pointer_move(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        if route.drag.as_ref().is_some_and(|drag| drag.locked) {
            return self.drag_by(route, event);
        }
        let Some((origin_x, origin_y, hit)) = route
            .drag
            .as_ref()
            .map(|drag| (drag.origin_x, drag.origin_y, drag.hit))
        else {
            return self.update_hover(event.x, event.y);
        };
        let past = past_slop(origin_x, origin_y, event.x, event.y);
        if let Some(target) = self.long_press_winner(route, &event) {
            return vec![Gesture::LongPress(target)];
        }
        if !past {
            return self.move_pressed(route, event);
        }
        let dx = event.x - origin_x;
        let dy = event.y - origin_y;
        if let Some(target) = route.pressed
            && self.claims_drag(target, dx, dy)
        {
            return self.lock_select(route, event, target);
        }
        if let Some(scroll) =
            hit.and_then(|id| self.scroll_for(id, dx, dy, Some(event.pointer), true))
        {
            self.arena.scroll_locks.insert(scroll, event.pointer);
            return self.lock_drag(route, event, scroll);
        }
        self.move_pressed(route, event)
    }

    pub(super) fn drag_by(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        let Some(drag) = route.drag.as_mut() else {
            return Vec::new();
        };
        let Some(scroll) = drag.scroll else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        vec![Gesture::DragBy(scroll, dx, dy)]
    }

    pub(super) fn lock_drag(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
        scroll: NodeId,
    ) -> Vec<Gesture> {
        let Some(drag) = route.drag.as_mut() else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        drag.locked = true;
        drag.scroll = Some(scroll);
        self.cancel_scroll_motion(scroll);
        self.note_scroll_phase(scroll, crate::ScrollPhase::Dragging, 0.0, 0.0);
        let mut gestures = Vec::new();
        if let Some(target) = route.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        gestures.push(Gesture::DragBegin(scroll));
        gestures.push(Gesture::DragBy(scroll, dx, dy));
        gestures
    }

    /// Selection drag keeps the press. The target is the pointer region.
    pub(super) fn lock_select(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
        target: NodeId,
    ) -> Vec<Gesture> {
        let Some(drag) = route.drag.as_mut() else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        drag.locked = true;
        drag.scroll = Some(target);
        vec![Gesture::DragBegin(target), Gesture::DragBy(target, dx, dy)]
    }

    pub(super) fn move_pressed(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        if let Some(drag) = route.drag.as_mut() {
            note_move(drag, event);
        }
        let Some(target) = route.pressed else {
            return Vec::new();
        };
        let still = self.point_hits_node(target, event.x, event.y);
        if still {
            Vec::new()
        } else {
            route.pressed = None;
            route.drag = None;
            vec![Gesture::Release(target)]
        }
    }

    pub(super) fn pointer_up(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        if route.drag.as_ref().is_some_and(|drag| drag.long_press) {
            let target = route.pressed.take();
            route.drag = None;
            route.contest = None;
            return target.map(Gesture::Release).into_iter().collect();
        }
        if route.drag.as_ref().is_some_and(|drag| drag.locked) {
            let scroll = route.drag.as_ref().and_then(|drag| drag.scroll);
            let speed = route.drag.as_ref().map(drag_velocity).unwrap_or((0.0, 0.0));
            route.drag = None;
            route.pressed = None;
            route.contest = None;
            self.unlock_pointer(event.pointer);
            return scroll
                .map(|id| {
                    if self.get(id).is_some_and(|node| node.kind == Kind::Scroll) {
                        let _ = self.arm_fling(id, speed.0, speed.1);
                        self.arm_overscroll_spring(id);
                        self.sync_scroll_phase(id);
                    }
                    vec![Gesture::DragEnd(id, speed.0, speed.1)]
                })
                .unwrap_or_default();
        }
        let hit = route.drag.as_ref().and_then(|drag| drag.hit);
        route.drag = None;
        route.contest = None;
        let Some(target) = route.pressed.take() else {
            return Vec::new();
        };
        let inside = self.point_hits_node(target, event.x, event.y);
        if !inside {
            return vec![Gesture::Release(target)];
        }
        if route.consumed_double {
            return vec![Gesture::Release(target)];
        }
        if let Some(config) = self.double_tap_config(target) {
            let waits = self
                .get(target)
                .is_some_and(|node| node.recognizers.tap || node.kind == Kind::PointerRegion);
            self.arena.pending_tap = Some(PendingTap {
                node: target,
                x: event.x,
                y: event.y,
                time_ms: event.time_ms,
                deadline_ms: waits.then_some(event.time_ms + config.window_ms),
                focus: self.focusable_of(target),
                window_ms: config.window_ms,
                slop: config.slop,
            });
            return vec![Gesture::Release(target)];
        }
        let mut gestures = vec![Gesture::Release(target), Gesture::Click(target)];
        if let Some(id) = hit
            && let Some(link) = self.link_at(id, event.x, event.y)
        {
            gestures.push(Gesture::ActivateLink { id, link });
        }
        let next = self.focusable_of(target);
        gestures.extend(self.move_focus(next));
        gestures
    }

    fn link_at(&self, id: NodeId, x: f32, y: f32) -> Option<u32> {
        let links = self.get(id)?.text()?.links.clone();
        if links.is_empty() {
            return None;
        }
        let unit = self.caret_at(id, x, y).ok()?.unit;
        links
            .iter()
            .find(|link| unit >= link.start && unit < link.end)
            .map(|link| link.id)
    }

    pub(super) fn pointer_secondary(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        match event.phase {
            Phase::Down => {
                let target = self
                    .hit_root(event.x, event.y)
                    .and_then(|id| self.region_of(id));
                route.secondary = target.map(|target| SecondaryPress {
                    target,
                    origin_x: event.x,
                    origin_y: event.y,
                    cancelled: false,
                });
                Vec::new()
            }
            Phase::Move => {
                if let Some(press) = route.secondary.as_mut()
                    && !press.cancelled
                    && past_slop(press.origin_x, press.origin_y, event.x, event.y)
                {
                    press.cancelled = true;
                }
                Vec::new()
            }
            Phase::Up => {
                let Some(press) = route.secondary.take() else {
                    return Vec::new();
                };
                if press.cancelled {
                    return Vec::new();
                }
                if self.point_hits_node(press.target, event.x, event.y) {
                    vec![Gesture::SecondaryClick(press.target)]
                } else {
                    Vec::new()
                }
            }
            Phase::Cancel | Phase::Wheel | Phase::Zoom => Vec::new(),
        }
    }

    pub(super) fn pointer_cancel(
        &mut self,
        route: &mut PointerRoute,
        pointer: PointerId,
    ) -> Vec<Gesture> {
        route.secondary = None;
        let mut gestures = Vec::new();
        if let Some(target) = route.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        if route.drag.as_ref().is_some_and(|drag| drag.locked)
            && let Some(scroll) = route.drag.as_ref().and_then(|drag| drag.scroll)
        {
            gestures.push(Gesture::DragEnd(scroll, 0.0, 0.0));
            self.arm_overscroll_spring(scroll);
        }
        route.drag = None;
        route.contest = None;
        self.unlock_pointer(pointer);
        gestures
    }

    fn long_press_winner(
        &mut self,
        route: &mut PointerRoute,
        event: &PointerEvent,
    ) -> Option<NodeId> {
        let claim = {
            let contest = route.contest.as_mut()?;
            let mut path = crate::HitPath::default();
            self.hit_path(event.x, event.y, &mut path);
            contest.on_pointer(event, &path)?
        };
        if !matches!(claim, crate::RecognizerClaim::LongPress) {
            return None;
        }
        let drag = route.drag.as_mut()?;
        if drag.long_press || drag.locked {
            return None;
        }
        drag.long_press = true;
        route.pressed
    }

    pub(super) fn unlock_pointer(&mut self, pointer: PointerId) {
        self.arena.scroll_locks.retain(|_, owner| *owner != pointer);
    }

    pub(super) fn pointer_wheel(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(hit) = self.hit_root(event.x, event.y) else {
            return Vec::new();
        };
        // Wheel sign is opposite the finger: positive dy increases the offset.
        // Only the scroll's declared axis is forwarded.
        let Some(id) = self.scroll_for(hit, -event.dx, -event.dy, None, false) else {
            return Vec::new();
        };
        let (dx, dy) = self.axis_of(id).filter(event.dx, event.dy);
        if dx == 0.0 && dy == 0.0 {
            return Vec::new();
        }
        self.merge_wheel(id, dx, dy);
        vec![Gesture::Wheel(id, dx, dy)]
    }

    /// Drops pointer routes whose target sits in `root`'s subtree.
    ///
    /// Inserting a row must not cancel a drag on the surrounding scroll.
    /// Removing that scroll, or the pressed region, does.
    pub(super) fn drop_routes_in(&mut self, root: NodeId) {
        let pointers: Vec<_> = self.arena.routes.keys().copied().collect();
        for pointer in pointers {
            let Some(route) = self.arena.routes.get(&pointer) else {
                continue;
            };
            let pressed = route.pressed;
            let secondary = route.secondary.as_ref().map(|press| press.target);
            let drag_scroll = route.drag.as_ref().and_then(|drag| drag.scroll);
            let drag_hit = route.drag.as_ref().and_then(|drag| drag.hit);
            let drag_locked = route.drag.as_ref().is_some_and(|drag| drag.locked);
            let drop_press = pressed.is_some_and(|id| self.contains(root, id));
            let drop_secondary = secondary.is_some_and(|id| self.contains(root, id));
            let drop_drag = drag_scroll.is_some_and(|id| self.contains(root, id))
                || (!drag_locked && drag_hit.is_some_and(|id| self.contains(root, id)));
            let Some(route) = self.arena.routes.get_mut(&pointer) else {
                continue;
            };
            if drop_press {
                route.pressed = None;
                if route.drag.as_ref().is_some_and(|drag| !drag.locked) {
                    route.drag = None;
                }
            }
            if drop_secondary {
                route.secondary = None;
            }
            if drop_drag {
                route.drag = None;
                route.pressed = None;
            }
            if route.is_empty() {
                self.arena.routes.remove(&pointer);
            }
        }
        let locked: Vec<_> = self.arena.scroll_locks.keys().copied().collect();
        for id in locked {
            if self.contains(root, id) {
                self.arena.scroll_locks.remove(&id);
            }
        }
    }

    pub(super) fn update_hover(&mut self, x: f32, y: f32) -> Vec<Gesture> {
        let next = self.hit_root(x, y);
        if self.arena.hovered == next {
            return Vec::new();
        }
        let mut gestures = Vec::new();
        if let Some(old) = self.arena.hovered.take() {
            gestures.push(Gesture::Leave(old));
        }
        if let Some(id) = next {
            self.arena.hovered = Some(id);
            gestures.push(Gesture::Hover(id));
        }
        gestures
    }

    pub(super) fn drain_pending(&mut self) -> Vec<Gesture> {
        std::mem::take(&mut self.arena.pending)
    }

    pub(super) fn move_focus(&mut self, next: Option<NodeId>) -> Vec<Gesture> {
        if next.is_some_and(|id| !self.inside_scope(id)) {
            return Vec::new();
        }
        if self.focused == next {
            return Vec::new();
        }
        let mut gestures = Vec::new();
        if let Some(old) = self.focused.take() {
            gestures.push(Gesture::Blur(old));
        }
        if let Some(id) = next {
            self.focused = Some(id);
            gestures.push(Gesture::Focus(id));
        }
        gestures
    }

    /// Scene point of the event [`Tree::pointer`] is handling.
    pub fn pointer_position(&self) -> (f32, f32) {
        self.arena.pointer_at
    }

    /// Restricts focus to `id` and its descendants. A focus outside the new
    /// scope is cleared.
    pub fn push_focus_scope(&mut self, id: NodeId) -> Result<Vec<Gesture>, Error> {
        self.node(id)?;
        self.scopes.push(id);
        if self
            .focused
            .is_some_and(|focused| !self.inside_scope(focused))
        {
            Ok(self.move_focus(None))
        } else {
            Ok(Vec::new())
        }
    }

    /// Drops the innermost scope. An empty stack stays empty.
    pub fn pop_focus_scope(&mut self) {
        self.scopes.pop();
    }

    /// Moves focus by tree order or by geometry, inside the active scope.
    pub fn focus_step(&mut self, direction: FocusDirection) -> Vec<Gesture> {
        let Some(root) = self.scopes.last().copied().or(self.root) else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        self.collect_focusable(root, &mut nodes);
        if nodes.is_empty() {
            return Vec::new();
        }
        if matches!(direction, FocusDirection::Next | FocusDirection::Previous)
            && nodes.iter().any(|id| self.focus_order(*id).is_some())
        {
            let mut keyed: Vec<(usize, NodeId)> = nodes.iter().copied().enumerate().collect();
            keyed.sort_by(|left, right| {
                let left_order = self.focus_order(left.1).unwrap_or(i32::MAX);
                let right_order = self.focus_order(right.1).unwrap_or(i32::MAX);
                left_order.cmp(&right_order).then(left.0.cmp(&right.0))
            });
            nodes = keyed.into_iter().map(|(_, id)| id).collect();
        }
        let next = match self.focused {
            None => nodes[0],
            Some(current) => match direction {
                FocusDirection::Next => step_index(&nodes, current, 1),
                FocusDirection::Previous => step_index(&nodes, current, -1),
                FocusDirection::Left
                | FocusDirection::Right
                | FocusDirection::Up
                | FocusDirection::Down => self
                    .geometric_focus(&nodes, current, direction)
                    .unwrap_or(current),
            },
        };
        self.move_focus(Some(next))
    }

    fn collect_focusable(&self, id: NodeId, out: &mut Vec<NodeId>) {
        if !self.inside_scope(id) {
            return;
        }
        let Some(node) = self.get(id) else {
            return;
        };
        if node.is_focusable() && node.w > 0.0 && node.h > 0.0 {
            out.push(id);
        }
        if node.kind == Kind::Clip && (node.w == 0.0 || node.h == 0.0) {
            return;
        }
        let mut child = node.first_child;
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            self.collect_focusable(current, out);
            child = next;
        }
    }

    fn focus_order(&self, id: NodeId) -> Option<i32> {
        self.get(id)?.semantics.as_ref()?.order
    }

    fn double_tap_config(&self, id: NodeId) -> Option<crate::DoubleTapConfig> {
        self.get(id)?.recognizers.double_tap
    }

    fn geometric_focus(
        &self,
        nodes: &[NodeId],
        current: NodeId,
        direction: FocusDirection,
    ) -> Option<NodeId> {
        let origin = self.rect(current).ok()?;
        let band_h = (origin.h * 0.5).max(1.0);
        let origin_mid_y = origin.y + origin.h * 0.5;
        let band_of = |rect: crate::Rect| -> i32 {
            let mid = rect.y + rect.h * 0.5;
            ((mid - origin_mid_y) / band_h).round() as i32
        };
        let others: Vec<(NodeId, crate::Rect)> = nodes
            .iter()
            .copied()
            .filter(|id| *id != current)
            .filter_map(|id| Some((id, self.rect(id).ok()?)))
            .collect();
        match direction {
            FocusDirection::Left | FocusDirection::Right => {
                let mut band: Vec<(f32, NodeId)> = others
                    .iter()
                    .filter(|(_, rect)| band_of(*rect) == 0)
                    .map(|(id, rect)| (rect.x, *id))
                    .collect();
                band.sort_by(|left, right| {
                    left.0
                        .partial_cmp(&right.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                if direction == FocusDirection::Right {
                    band.into_iter()
                        .find(|(x, _)| *x > origin.x + 0.5)
                        .map(|(_, id)| id)
                } else {
                    band.into_iter()
                        .filter(|(x, _)| *x + 0.5 < origin.x)
                        .next_back()
                        .map(|(_, id)| id)
                }
            }
            FocusDirection::Up | FocusDirection::Down => {
                let step = if direction == FocusDirection::Up {
                    -1
                } else {
                    1
                };
                let mut best: Option<i32> = None;
                for (_, rect) in &others {
                    let delta = band_of(*rect);
                    if delta == 0 || delta.signum() != step {
                        continue;
                    }
                    best = Some(match best {
                        Some(current_best) if delta.abs() < current_best.abs() => delta,
                        Some(current_best) => current_best,
                        None => delta,
                    });
                }
                let band = best?;
                others
                    .iter()
                    .filter(|(_, rect)| band_of(*rect) == band)
                    .max_by(|left, right| {
                        overlap_x(origin, left.1)
                            .partial_cmp(&overlap_x(origin, right.1))
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| {
                                center_distance(origin, right.1)
                                    .partial_cmp(&center_distance(origin, left.1))
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                    })
                    .map(|(id, _)| *id)
            }
            FocusDirection::Next | FocusDirection::Previous => None,
        }
    }

    pub(super) fn blur_contained(&mut self, root: NodeId) -> Vec<Gesture> {
        let Some(focused) = self.focused else {
            return Vec::new();
        };
        match self.is_under(focused, root) {
            Ok(true) => self.move_focus(None),
            _ => Vec::new(),
        }
    }
}

fn step_index(nodes: &[NodeId], current: NodeId, delta: isize) -> NodeId {
    let Some(index) = nodes.iter().position(|id| *id == current) else {
        return nodes[0];
    };
    let len = nodes.len() as isize;
    let next = (index as isize + delta).rem_euclid(len) as usize;
    nodes[next]
}

fn overlap_x(current: crate::Rect, other: crate::Rect) -> f32 {
    let start = current.x.max(other.x);
    let end = (current.x + current.w).min(other.x + other.w);
    (end - start).max(0.0)
}

fn center_distance(current: crate::Rect, other: crate::Rect) -> f32 {
    let dx = (other.x + other.w * 0.5) - (current.x + current.w * 0.5);
    let dy = (other.y + other.h * 0.5) - (current.y + current.h * 0.5);
    dx * dx + dy * dy
}

fn beyond(x0: f32, y0: f32, x1: f32, y1: f32, slop: f32) -> bool {
    let dx = x1 - x0;
    let dy = y1 - y0;
    dx * dx + dy * dy > slop * slop
}

fn past_slop(origin_x: f32, origin_y: f32, x: f32, y: f32) -> bool {
    let dx = x - origin_x;
    let dy = y - origin_y;
    dx * dx + dy * dy > DRAG_SLOP * DRAG_SLOP
}

fn note_move(drag: &mut DragState, event: PointerEvent) -> (f32, f32) {
    let dx = event.x - drag.last_x;
    let dy = event.y - drag.last_y;
    drag.previous_move = drag.latest_move;
    drag.latest_move = Some(Sample {
        x: event.x,
        y: event.y,
        time_ms: event.time_ms,
    });
    drag.velocity.push(event.x, event.y, event.time_ms);
    drag.last_x = event.x;
    drag.last_y = event.y;
    (dx, dy)
}

fn drag_velocity(drag: &DragState) -> (f32, f32) {
    let from_ring = drag.velocity.velocity();
    if from_ring != (0.0, 0.0) {
        return from_ring;
    }
    let (Some(previous), Some(latest)) = (drag.previous_move, drag.latest_move) else {
        return (0.0, 0.0);
    };
    let dt = latest.time_ms - previous.time_ms;
    if !dt.is_finite() || dt <= 0.0 {
        return (0.0, 0.0);
    }
    (
        (f64::from(latest.x - previous.x) / dt) as f32,
        (f64::from(latest.y - previous.y) / dt) as f32,
    )
}

fn text_select_axis(node: &super::node::Node) -> Option<crate::DragAxis> {
    let editing = node.text().is_some_and(|text| text.editor.is_some());
    if node.drag_claim == super::DragClaim::None && !editing {
        return None;
    }
    Some(match node.drag_claim {
        super::DragClaim::Horizontal => crate::DragAxis::Horizontal,
        super::DragClaim::Free | super::DragClaim::None => crate::DragAxis::Free,
    })
}
