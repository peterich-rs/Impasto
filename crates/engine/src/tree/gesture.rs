//! Press, drag, focus, hover, wheel, and the right button.
//!
//! The right button keeps its own press. It never writes [`Tree::pressed`] or [`Tree::drag`].

use std::collections::HashMap;

use super::node::Kind;
use super::{FocusDirection, Gesture, NodeId, Tree};
use crate::{
    Error,
    input::{Button, Phase, PointerEvent, PointerId, PointerKind, PointerSample, drag_slop},
};

/// Routes captured at pointer down. Not a gesture contest: one pointer has one route.
/// A later structure change does not drop a route unless the target node itself is removed.
pub(super) struct PointerTable {
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

impl Default for PointerTable {
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
    kind: PointerKind,
    /// `Some` schedules [`Gesture::Click`] when [`Tree::tick`] passes this time.
    deadline_ms: Option<f64>,
    focus: Option<NodeId>,
    window_ms: f64,
    slop: f32,
}

impl PendingTap {
    fn matches(&self, event: &PointerEvent) -> bool {
        event.kind == self.kind
            && (0.0..=self.window_ms).contains(&(event.time_ms - self.time_ms))
            && !beyond(self.x, self.y, event.x, event.y, self.slop)
    }
}

impl PointerTable {
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
            if route
                .long_press
                .as_ref()
                .is_some_and(|watch| watch.target.index == index)
            {
                route.long_press = None;
            }
            !route.is_empty()
        });
        self.scroll_locks.retain(|node, _| node.index != index);
    }
}

pub(super) struct DragState {
    /// Deepest node under the pointer at press time.
    pub(super) hit: Option<NodeId>,
    /// Scroll or selection target after the gesture locks.
    pub(super) scroll: Option<NodeId>,
    pub(super) locked: bool,
    /// The long-press watch already won. Lift releases without a click.
    /// Unclamped content position for a locked scroll. Later samples continue from here.
    pub(super) desired_x: f32,
    pub(super) desired_y: f32,
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) last_x: f32,
    pub(super) last_y: f32,
    pub(super) velocity: crate::input::VelocityRing,
    /// One "scroll not claimed" note per press. Later moves stay quiet.
    miss_noted: bool,
    /// Last time a locked drag logged an offset rebase.
    noted_ms: f64,
}

/// Press, drag, and right button for one [`crate::input::PointerId`].
#[derive(Default)]
pub(super) struct PointerRoute {
    pub(super) pressed: Option<NodeId>,
    pub(super) secondary: Option<SecondaryPress>,
    pub(super) drag: Option<DragState>,
    /// Still-finger long press. Cleared when the drag locks or the finger leaves the slop.
    pub(super) long_press: Option<LongPressWatch>,
    /// Release time of the first tap, if this press can complete a double tap.
    pub(super) first_tap_ms: Option<f64>,
}

/// Time and slop for one long press. The duration is [`crate::input::LONG_PRESS_MS`].
pub(super) struct LongPressWatch {
    pub(super) target: NodeId,
    pub(super) down_ms: f64,
    origin_x: f32,
    origin_y: f32,
    pub(super) cancelled: bool,
    pub(super) fired: bool,
}

impl LongPressWatch {
    fn past_slop(&self, x: f32, y: f32, kind: PointerKind) -> bool {
        beyond(self.origin_x, self.origin_y, x, y, drag_slop(kind))
    }
}

impl PointerRoute {
    fn long_press_deadline(&self) -> Option<f64> {
        let watch = self.long_press.as_ref()?;
        (!watch.cancelled && !watch.fired && !self.drag.as_ref().is_some_and(|drag| drag.locked))
            .then_some(watch.down_ms + crate::input::LONG_PRESS_MS)
            .filter(|deadline| deadline.is_finite())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pressed.is_none()
            && self.secondary.is_none()
            && self.drag.is_none()
            && self.long_press.is_none()
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
    /// the pointer’s slop keeps that press. Past the slop, a region that
    /// [`Tree::set_drag_claim`]s the dominant axis locks as a selection drag.
    /// Otherwise the scroll chain does: a scroll at its edge yields to an
    /// ancestor that can still move on that axis. One scroll keeps one locked
    /// pointer. A lift inside the pressed region, with no lock, is
    /// [`Gesture::Release`] then [`Gesture::Click`], and that click is what moves
    /// focus. A locked drag does not. Does not lay out.
    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Gesture> {
        self.pointer_with_history(event, &[])
    }

    pub fn pointer_with_history(
        &mut self,
        event: PointerEvent,
        history: &[PointerSample],
    ) -> Vec<Gesture> {
        if self.pick_remote_pointer(event) {
            return Vec::new();
        }
        self.arena.pointer_at = (event.x, event.y);
        let mut gestures = self.drain_pending();
        if self.debug.pointer_trace {
            self.note_pointer(event.x, event.y, phase_name(event.phase));
        }
        if !self.laid_out {
            return self.finish_pointer(gestures);
        }
        let mut route = self.arena.routes.remove(&event.pointer).unwrap_or_default();
        let pressed = route.pressed;
        let hovered = self.arena.hovered;
        if event.phase == Phase::Move {
            for sample in history {
                if sample.time_ms <= event.time_ms {
                    if let Some(drag) = route.drag.as_mut() {
                        drag.velocity.push(sample.x, sample.y, sample.time_ms);
                    }
                    if let Some(watch) = route.long_press.as_mut()
                        && watch.past_slop(sample.x, sample.y, event.kind)
                    {
                        watch.cancelled = true;
                    }
                }
            }
        }
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
        if pressed != route.pressed {
            self.invalidate_style_deps(crate::style::DepMask::ACTIVE);
        }
        if hovered != self.arena.hovered {
            self.invalidate_style_deps(crate::style::DepMask::HOVER);
        }
        if !route.is_empty() {
            self.arena.routes.insert(event.pointer, route);
        }
        gestures.extend(produced);
        self.finish_pointer(gestures)
    }

    /// The inspector's probe mode still runs hit-testing. Listeners must not
    /// see the gestures, or picking an element would click the app.
    fn finish_pointer(&self, gestures: Vec<Gesture>) -> Vec<Gesture> {
        if self.debug.suppress_dispatch {
            Vec::new()
        } else {
            gestures
        }
    }

    /// Primary press only. Callers keep other buttons out of this path.
    pub(super) fn pointer_down(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        route.drag = None;
        route.pressed = None;
        route.long_press = None;
        route.first_tap_ms = None;
        let hit = self.hit_root(event.x, event.y);
        let pressed = hit.and_then(|id| self.region_of(id));
        let mut early = self.check_second_tap(route, &event, pressed);
        let scroll = hit.and_then(|id| self.drag_of(id));
        route.pressed = pressed;
        if pressed.is_some() || scroll.is_some() {
            let mut velocity = crate::input::VelocityRing::new();
            velocity.push(event.x, event.y, event.time_ms);
            route.drag = Some(DragState {
                hit,
                scroll,
                locked: false,
                desired_x: 0.0,
                desired_y: 0.0,
                origin_x: event.x,
                origin_y: event.y,
                last_x: event.x,
                last_y: event.y,
                velocity,
                miss_noted: false,
                noted_ms: f64::NEG_INFINITY,
            });
        }
        if let Some(scroll) = scroll {
            let phase = self
                .scroll_phase(scroll)
                .map(|phase| phase.name())
                .unwrap_or("none");
            let offset_y = self.scroll_offset(scroll).map(|pair| pair.1).unwrap_or(0.0);
            tracing::info!(
                target: "impasto::gesture",
                pointer = event.pointer.0,
                pressed = pressed.map(NodeId::to_raw).unwrap_or(0),
                scroll = scroll.to_raw(),
                phase,
                offset_y,
                "pointer down"
            );
        }
        route.long_press = self.long_press_watch(hit, &event);
        early.extend(pressed.into_iter().map(Gesture::Press));
        early
    }

    fn check_second_tap(
        &mut self,
        route: &mut PointerRoute,
        event: &PointerEvent,
        target: Option<NodeId>,
    ) -> Vec<Gesture> {
        let Some(pending) = self.arena.pending_tap.take() else {
            return Vec::new();
        };
        if target == Some(pending.node) && pending.matches(event) {
            route.first_tap_ms = Some(pending.time_ms);
            self.arena.pending_tap = Some(pending);
            return Vec::new();
        }
        if pending.deadline_ms.is_some() {
            return self.finish_tap(pending);
        }
        Vec::new()
    }

    /// Click scheduled by a tap that was waiting for a double tap, and a still-finger long press.
    pub fn tick(&mut self, now_ms: f64) -> Vec<Gesture> {
        let mut gestures = self.tick_long_press(now_ms);
        let Some(pending) = self.arena.pending_tap.take() else {
            return gestures;
        };
        if pending
            .deadline_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            gestures.extend(self.finish_tap(pending));
            return gestures;
        }
        if pending.deadline_ms.is_none() && now_ms - pending.time_ms > pending.window_ms {
            return gestures;
        }
        self.arena.pending_tap = Some(pending);
        gestures
    }

    fn finish_tap(&mut self, pending: PendingTap) -> Vec<Gesture> {
        let mut gestures = vec![Gesture::Click(pending.node)];
        gestures.extend(self.move_focus(pending.focus));
        gestures
    }

    /// Earliest gesture expiration on the pointer/display clock.
    pub fn next_gesture_deadline_ms(&self) -> Option<f64> {
        self.arena
            .routes
            .values()
            .filter_map(PointerRoute::long_press_deadline)
            .chain(
                self.arena
                    .pending_tap
                    .as_ref()
                    .and_then(|tap| tap.deadline_ms),
            )
            .filter(|deadline| deadline.is_finite())
            .min_by(f64::total_cmp)
    }

    fn pointer_zoom(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(hit) = self.hit_root(event.x, event.y) else {
            return Vec::new();
        };
        let Some(id) = self.zoom_target(hit) else {
            return Vec::new();
        };
        vec![Gesture::Zoom {
            id,
            scale: event.zoom,
        }]
    }

    fn zoom_target(&self, mut id: NodeId) -> Option<NodeId> {
        use super::node::Role;

        let mut transform = None;
        while let Some(node) = self.get(id) {
            if node.role.contains(Role::ZOOM) {
                return Some(id);
            }
            if transform.is_none() && node.role.contains(Role::TRANSFORM) {
                transform = Some(id);
            }
            let Some(parent) = node.parent else {
                break;
            };
            id = parent;
        }
        transform
    }

    fn long_press_watch(
        &self,
        hit: Option<NodeId>,
        event: &PointerEvent,
    ) -> Option<LongPressWatch> {
        let mut id = hit?;
        loop {
            let node = self.get(id)?;
            if node.recognizers.long_press {
                return Some(LongPressWatch {
                    target: id,
                    down_ms: event.time_ms,
                    origin_x: event.x,
                    origin_y: event.y,
                    cancelled: false,
                    fired: false,
                });
            }
            id = node.parent?;
        }
    }

    fn tick_long_press(&mut self, now_ms: f64) -> Vec<Gesture> {
        self.arena
            .routes
            .values_mut()
            .filter_map(|route| fire_long_press(route, now_ms).map(Gesture::LongPress))
            .collect()
    }

    fn check_long_press_position(&self, route: &mut PointerRoute, event: PointerEvent) {
        if let Some(watch) = route.long_press.as_mut()
            && !watch.fired
            && !watch.cancelled
            && (watch.past_slop(event.x, event.y, event.kind)
                || !self.point_hits_node(watch.target, event.x, event.y))
        {
            watch.cancelled = true;
        }
    }

    pub fn set_recognizers(
        &mut self,
        id: NodeId,
        set: crate::input::RecognizerSet,
    ) -> Result<(), Error> {
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
        if route.long_press.as_ref().is_some_and(|watch| watch.fired) {
            return Vec::new();
        }
        self.check_long_press_position(route, event);
        if let Some(target) = fire_long_press(route, event.time_ms) {
            return vec![Gesture::LongPress(target)];
        }
        let Some((origin_x, origin_y, hit)) = route
            .drag
            .as_ref()
            .map(|drag| (drag.origin_x, drag.origin_y, drag.hit))
        else {
            return if event.kind == PointerKind::Touch {
                Vec::new()
            } else {
                self.update_hover(event.x, event.y)
            };
        };
        if !beyond(origin_x, origin_y, event.x, event.y, drag_slop(event.kind)) {
            return self.move_pressed(route, event);
        }
        let dx = event.x - origin_x;
        let dy = event.y - origin_y;
        if let Some(target) = route.pressed
            && self.claims_drag(target, dx, dy)
        {
            return self.lock_select(route, event, target);
        }
        let from_hit = hit
            .filter(|id| self.get(*id).is_some())
            .and_then(|id| self.scroll_for(id, dx, dy, Some(event.pointer), true));
        let from_scroll = route
            .drag
            .as_ref()
            .and_then(|drag| drag.scroll)
            .and_then(|id| self.scroll_for(id, dx, dy, Some(event.pointer), true));
        if let Some(scroll) = from_hit.or(from_scroll) {
            self.arena.scroll_locks.insert(scroll, event.pointer);
            return self.lock_drag(route, event, scroll);
        }
        if route.drag.as_ref().is_some_and(|drag| !drag.miss_noted) {
            let reason = self.claim_miss_reason(hit, event.pointer);
            let scroll = hit.and_then(|id| self.drag_of(id)).map(NodeId::to_raw);
            if let Some(drag) = route.drag.as_mut() {
                drag.miss_noted = true;
            }
            tracing::warn!(
                target: "impasto::gesture",
                reason,
                pointer = event.pointer.0,
                scroll = scroll.unwrap_or(0),
                dx,
                dy,
                "scroll not claimed"
            );
        }
        self.move_pressed(route, event)
    }

    /// Why [`Tree::scroll_for`] declined this drag. The innermost scroll is enough
    /// to tell a stuck lock from an edge or a miss.
    fn claim_miss_reason(&self, hit: Option<NodeId>, pointer: PointerId) -> &'static str {
        let Some(hit) = hit else {
            return "no-hit";
        };
        let Some(scroll) = self.drag_of(hit) else {
            return "no-scroll";
        };
        if crate::input::gesture::scroll_locked_by_other(
            &self.arena.scroll_locks,
            scroll,
            Some(pointer),
        ) {
            "locked-by-other"
        } else {
            "not-claimed"
        }
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
        let scroll_kind = self
            .get(scroll)
            .is_some_and(|node| node.kind == Kind::Scroll);
        if scroll_kind {
            drag.desired_x -= dx;
            drag.desired_y -= dy;
            let desired_x = drag.desired_x;
            let desired_y = drag.desired_y;
            if let Ok(sample) = self.apply_drag_desired(scroll, desired_x, desired_y) {
                self.note_edge_pull(scroll, desired_y, sample.offset_y);
                self.note_overscroll_pull(scroll);
            }
        }
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
        note_move(drag, event);
        let (dx, dy) = drag_after_slop(drag, event);
        drag.locked = true;
        drag.scroll = Some(scroll);
        if let Some(watch) = route.long_press.as_mut() {
            watch.cancelled = true;
        }
        let (ox, oy) = self.scroll_offset(scroll).unwrap_or((0.0, 0.0));
        let phase = self
            .scroll_phase(scroll)
            .map(|phase| phase.name())
            .unwrap_or("none");
        tracing::info!(
            target: "impasto::gesture",
            pointer = event.pointer.0,
            scroll = scroll.to_raw(),
            phase,
            dx,
            dy,
            offset_y = oy,
            "scroll locked"
        );
        drag.desired_x = ox - dx;
        drag.desired_y = oy - dy;
        let desired_x = drag.desired_x;
        let desired_y = drag.desired_y;
        if let Ok(sample) = self.apply_drag_desired(scroll, desired_x, desired_y) {
            self.note_edge_pull(scroll, desired_y, sample.offset_y);
            self.note_overscroll_pull(scroll);
        }
        self.cancel_scroll_motion(scroll);
        self.note_scroll_phase(scroll, crate::motion::ScrollPhase::Dragging, 0.0, 0.0);
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
        note_move(drag, event);
        let (dx, dy) = drag_after_slop(drag, event);
        drag.locked = true;
        drag.scroll = Some(target);
        if let Some(watch) = route.long_press.as_mut() {
            watch.cancelled = true;
        }
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
            return Vec::new();
        }
        // The row can be shorter than the slop. Releasing it must not throw away
        // the scroll while the finger is still inside the scroller from pointer down.
        let tracked = route
            .drag
            .as_ref()
            .filter(|drag| !drag.locked)
            .and_then(|drag| drag.scroll);
        let keep = tracked.is_some_and(|scroll| self.point_hits_node(scroll, event.x, event.y));
        if let Some(drag) = route.drag.as_ref().filter(|drag| !drag.locked) {
            let dx = event.x - drag.origin_x;
            let dy = event.y - drag.origin_y;
            let dist = (dx * dx + dy * dy).sqrt();
            let scroll = tracked.or_else(|| self.drag_of(target));
            let phase = scroll
                .and_then(|id| self.scroll_phase(id).ok())
                .map(|phase| phase.name())
                .unwrap_or("none");
            let reason = if dist < drag_slop(event.kind) {
                "left-before-slop"
            } else {
                "left-after-miss"
            };
            if keep {
                tracing::info!(
                    target: "impasto::gesture",
                    reason,
                    pointer = event.pointer.0,
                    pressed = target.to_raw(),
                    scroll = scroll.map(NodeId::to_raw).unwrap_or(0),
                    phase,
                    dist,
                    dx,
                    dy,
                    "press released"
                );
            } else {
                tracing::warn!(
                    target: "impasto::gesture",
                    reason,
                    pointer = event.pointer.0,
                    pressed = target.to_raw(),
                    scroll = scroll.map(NodeId::to_raw).unwrap_or(0),
                    phase,
                    dist,
                    dx,
                    dy,
                    "gesture dropped"
                );
            }
        }
        route.pressed = None;
        if !keep {
            route.drag = None;
        }
        vec![Gesture::Release(target)]
    }

    pub(super) fn pointer_up(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        let first_tap_ms = route.first_tap_ms.take();
        self.check_long_press_position(route, event);
        let long_press = fire_long_press(route, event.time_ms);
        if route.long_press.as_ref().is_some_and(|watch| watch.fired) {
            let release = route.pressed.take().map(Gesture::Release);
            route.drag = None;
            route.long_press = None;
            return long_press
                .map(Gesture::LongPress)
                .into_iter()
                .chain(release)
                .collect();
        }
        if route.drag.as_ref().is_some_and(|drag| drag.locked) {
            let scroll = route.drag.as_ref().and_then(|drag| drag.scroll);
            let speed = route
                .drag
                .as_ref()
                .map(|drag| drag.velocity.velocity_at(event.time_ms))
                .unwrap_or((0.0, 0.0));
            let mut gestures: Vec<_> = route
                .pressed
                .take()
                .map(Gesture::Release)
                .into_iter()
                .collect();
            route.drag = None;
            route.long_press = None;
            self.unlock_pointer(event.pointer);
            if let Some(id) = scroll {
                let offset_y = self.scroll_offset(id).map(|pair| pair.1).unwrap_or(0.0);
                tracing::info!(
                    target: "impasto::gesture",
                    pointer = event.pointer.0,
                    scroll = id.to_raw(),
                    vx = speed.0,
                    vy = speed.1,
                    offset_y,
                    "pointer up"
                );
            }
            if let Some(id) = scroll {
                if self.get(id).is_some_and(|node| node.kind == Kind::Scroll) {
                    self.commit_edge_pull(id);
                    let _ = self.arm_fling(id, speed.0, speed.1);
                    self.arm_overscroll_spring(id, speed.0, speed.1);
                    self.sync_scroll_phase(id);
                }
                gestures.push(Gesture::DragEnd(id, speed.0, speed.1));
            }
            return gestures;
        }
        let hit = route.drag.as_ref().and_then(|drag| drag.hit);
        route.drag = None;
        route.long_press = None;
        let Some(target) = route.pressed.take() else {
            return Vec::new();
        };
        let inside = self.point_hits_node(target, event.x, event.y);
        if !inside {
            return vec![Gesture::Release(target)];
        }
        let mut gestures = vec![Gesture::Release(target)];
        let config = self.double_tap_config(target);
        if let Some(pending) = self.arena.pending_tap.take() {
            if first_tap_ms == Some(pending.time_ms)
                && target == pending.node
                && pending.matches(&event)
            {
                gestures.push(Gesture::DoubleTap(target));
                return gestures;
            }
            if config.is_some() {
                if pending.deadline_ms.is_some() {
                    gestures.extend(self.finish_tap(pending));
                }
            } else {
                self.arena.pending_tap = Some(pending);
            }
        }
        if let Some(config) = config {
            let waits = self
                .get(target)
                .is_some_and(|node| node.recognizers.tap || node.is_pointer());
            self.arena.pending_tap = Some(PendingTap {
                node: target,
                x: event.x,
                y: event.y,
                time_ms: event.time_ms,
                kind: event.kind,
                deadline_ms: waits.then_some(event.time_ms + config.window_ms),
                focus: self.focusable_of(target),
                window_ms: config.window_ms,
                slop: config.slop,
            });
            return gestures;
        }
        gestures.push(Gesture::Click(target));
        if let Some(id) = hit
            && let Some(link) = self.link_at(id, event.x, event.y)
        {
            gestures.push(Gesture::ActivateLink { id, link });
        }
        let next = self.focusable_of(target);
        gestures.extend(self.move_focus(next));
        gestures
    }

    fn link_at(&mut self, id: NodeId, x: f32, y: f32) -> Option<u32> {
        let links = std::sync::Arc::clone(&self.get(id)?.text()?.links);
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
                    && beyond(
                        press.origin_x,
                        press.origin_y,
                        event.x,
                        event.y,
                        drag_slop(event.kind),
                    )
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
        if let Some(drag) = route.drag.as_ref() {
            tracing::info!(
                target: "impasto::gesture",
                pointer = pointer.0,
                scroll = drag.scroll.map(NodeId::to_raw).unwrap_or(0),
                locked = drag.locked,
                "pointer cancel"
            );
        }
        route.secondary = None;
        route.first_tap_ms = None;
        self.unlock_pointer(pointer);
        let mut gestures = Vec::new();
        if let Some(target) = route.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        if route.drag.as_ref().is_some_and(|drag| drag.locked)
            && let Some(scroll) = route.drag.as_ref().and_then(|drag| drag.scroll)
        {
            gestures.push(Gesture::DragEnd(scroll, 0.0, 0.0));
            self.arm_overscroll_spring(scroll, 0.0, 0.0);
            self.sync_scroll_phase(scroll);
        }
        route.drag = None;
        route.long_press = None;
        gestures
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
        let (x, y) = self.scroll_offset(id).unwrap_or((0.0, 0.0));
        let _ = self.resolve_scroll(id, x + dx, y + dy);
        self.merge_wheel(id, dx, dy);
        vec![Gesture::Wheel(id, dx, dy)]
    }

    /// A programmatic write during a drag continues from the requested position.
    pub(super) fn rebase_drag_desired(&mut self, id: NodeId, x: f32, y: f32) {
        let now = self.clock_ms;
        for route in self.arena.routes.values_mut() {
            let Some(drag) = route.drag.as_mut() else {
                continue;
            };
            if drag.locked && drag.scroll == Some(id) {
                let jump_x = (drag.desired_x - x).abs();
                let jump_y = (drag.desired_y - y).abs();
                if (jump_x > 0.5 || jump_y > 0.5) && now - drag.noted_ms > 200.0 {
                    drag.noted_ms = now;
                    tracing::warn!(
                        target: "impasto::gesture",
                        reason = "offset-rebase",
                        scroll = id.to_raw(),
                        from_y = drag.desired_y,
                        to_y = y,
                        jump_y,
                        "drag desired overwritten"
                    );
                }
                drag.desired_x = x;
                drag.desired_y = y;
            }
        }
    }

    /// Drops pointer routes whose target sits in `root`'s subtree.
    ///
    /// Inserting a row must not cancel a drag on the surrounding scroll.
    /// Removing that scroll does. Removing the pressed row only cancels the
    /// click; the drag stays if the scroll is still in the tree.
    pub(super) fn drop_routes_in(&mut self, root: NodeId) {
        let gestures = self.cancel_routes_in(root);
        self.arena.pending.extend(gestures);
    }

    fn cancel_routes_in(&mut self, root: NodeId) -> Vec<Gesture> {
        let mut gestures = Vec::new();
        if let Some(hovered) = self.arena.hovered
            && self.contains(root, hovered)
        {
            self.arena.hovered = None;
            self.invalidate_style_deps(crate::style::DepMask::HOVER);
            gestures.push(Gesture::Leave(hovered));
        }
        if self
            .arena
            .pending_tap
            .as_ref()
            .is_some_and(|tap| self.contains(root, tap.node))
        {
            self.arena.pending_tap = None;
        }

        let mut routes = std::mem::take(&mut self.arena.routes);
        let mut released = false;
        routes.retain(|&pointer, route| {
            let drag_target = route
                .drag
                .as_ref()
                .and_then(|drag| drag.scroll.or(drag.hit));
            let drop_drag = drag_target.is_some_and(|id| self.contains(root, id));
            if let Some(pressed) = route.pressed
                && (drop_drag || self.contains(root, pressed))
            {
                route.pressed = None;
                route.first_tap_ms = None;
                released = true;
                gestures.push(Gesture::Release(pressed));
            }
            if route
                .secondary
                .as_ref()
                .is_some_and(|press| self.contains(root, press.target))
            {
                route.secondary = None;
            }
            if drop_drag
                || route
                    .long_press
                    .as_ref()
                    .is_some_and(|watch| self.contains(root, watch.target))
            {
                route.long_press = None;
            }
            if drop_drag {
                self.unlock_pointer(pointer);
                if let Some(drag) = route.drag.take()
                    && drag.locked
                    && let Some(scroll) = drag.scroll
                {
                    gestures.push(Gesture::DragEnd(scroll, 0.0, 0.0));
                    self.sync_scroll_phase(scroll);
                }
            } else if let Some(drag) = route.drag.as_mut()
                && drag.hit.is_some_and(|hit| self.contains(root, hit))
            {
                drag.hit = None;
            }
            !route.is_empty()
        });
        self.arena.routes = routes;
        if released {
            self.invalidate_style_deps(crate::style::DepMask::ACTIVE);
        }
        gestures
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
            self.mark_semantics_chain(old);
            gestures.push(Gesture::Blur(old));
        }
        if let Some(id) = next {
            self.focused = Some(id);
            self.mark_semantics_chain(id);
            gestures.push(Gesture::Focus(id));
        }
        self.invalidate_style_deps(crate::style::DepMask::FOCUS);
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
    pub fn pop_focus_scope(&mut self) -> bool {
        self.scopes.pop().is_some()
    }

    /// Removes this scope and reports whether it was the active scope.
    pub fn remove_focus_scope(&mut self, id: NodeId) -> bool {
        let Some(index) = self.scopes.iter().rposition(|scope| *scope == id) else {
            return false;
        };
        let was_active = index + 1 == self.scopes.len();
        self.scopes.remove(index);
        was_active
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
        if node.role.contains(super::node::Role::CLIP) && (node.w == 0.0 || node.h == 0.0) {
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

    fn double_tap_config(&self, id: NodeId) -> Option<crate::input::DoubleTapConfig> {
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
        let band_of = |rect: crate::tree::Rect| -> i32 {
            let mid = rect.y + rect.h * 0.5;
            {
                let band = ((mid - origin_mid_y) / band_h).round();
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a focus band is a small signed row distance"
                )]
                {
                    band as i32
                }
            }
        };
        let others: Vec<(NodeId, crate::tree::Rect)> = nodes
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
                        .rfind(|(x, _)| *x + 0.5 < origin.x)
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

    /// Drop press, drag, hover, and focus inside a `display: none` subtree.
    pub(super) fn conceal(&mut self, id: NodeId) -> Vec<Gesture> {
        let mut gestures = self.blur_contained(id);
        gestures.extend(self.cancel_routes_in(id));
        gestures
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

fn overlap_x(current: crate::tree::Rect, other: crate::tree::Rect) -> f32 {
    let start = current.x.max(other.x);
    let end = (current.x + current.w).min(other.x + other.w);
    (end - start).max(0.0)
}

fn center_distance(current: crate::tree::Rect, other: crate::tree::Rect) -> f32 {
    let dx = (other.x + other.w * 0.5) - (current.x + current.w * 0.5);
    let dy = (other.y + other.h * 0.5) - (current.y + current.h * 0.5);
    dx * dx + dy * dy
}

fn beyond(x0: f32, y0: f32, x1: f32, y1: f32, slop: f32) -> bool {
    let dx = x1 - x0;
    let dy = y1 - y0;
    dx * dx + dy * dy > slop * slop
}

fn fire_long_press(route: &mut PointerRoute, now_ms: f64) -> Option<NodeId> {
    if !now_ms.is_finite() || now_ms < route.long_press_deadline()? {
        return None;
    }
    let watch = route.long_press.as_mut()?;
    watch.fired = true;
    Some(watch.target)
}

fn note_move(drag: &mut DragState, event: PointerEvent) -> (f32, f32) {
    let dx = event.x - drag.last_x;
    let dy = event.y - drag.last_y;
    drag.velocity.push(event.x, event.y, event.time_ms);
    drag.last_x = event.x;
    drag.last_y = event.y;
    (dx, dy)
}

fn drag_after_slop(drag: &DragState, event: PointerEvent) -> (f32, f32) {
    let x = event.x - drag.origin_x;
    let y = event.y - drag.origin_y;
    let distance = x.hypot(y);
    let remaining = (distance - drag_slop(event.kind)).max(0.0);
    (x / distance * remaining, y / distance * remaining)
}

/// Debug label for [`crate::input::PointerEvent::phase`].
fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Down => "down",
        Phase::Move => "move",
        Phase::Up => "up",
        Phase::Cancel => "cancel",
        Phase::Wheel => "wheel",
        Phase::Zoom => "zoom",
    }
}
