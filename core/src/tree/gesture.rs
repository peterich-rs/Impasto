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
            pointer_at: (0.0, 0.0),
        }
    }
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
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) last_x: f32,
    pub(super) last_y: f32,
    pub(super) previous_move: Option<Sample>,
    pub(super) latest_move: Option<Sample>,
}

/// Press, drag, and right button for one [`crate::PointerId`].
pub(super) struct PointerRoute {
    pub(super) pressed: Option<NodeId>,
    pub(super) secondary: Option<SecondaryPress>,
    pub(super) drag: Option<DragState>,
}

impl PointerRoute {
    pub(super) fn is_empty(&self) -> bool {
        self.pressed.is_none() && self.secondary.is_none() && self.drag.is_none()
    }
}

impl Default for PointerRoute {
    fn default() -> Self {
        Self {
            pressed: None,
            secondary: None,
            drag: None,
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
        self.node_mut(id)?.focusable = focusable;
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
        Ok(self.node(id)?.focusable)
    }

    /// Moves focus to `id` when it is marked focusable. The returned transition
    /// is empty when `id` is already focused.
    pub fn focus(&mut self, id: NodeId) -> Result<Vec<Gesture>, Error> {
        if !self.node(id)?.focusable {
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
        let hit = self.hit_root(event.x, event.y);
        let pressed = hit.and_then(|id| self.region_of(id));
        let scroll = hit.and_then(|id| self.drag_of(id));
        route.pressed = pressed;
        if pressed.is_some() || scroll.is_some() {
            route.drag = Some(DragState {
                hit,
                scroll,
                locked: false,
                origin_x: event.x,
                origin_y: event.y,
                last_x: event.x,
                last_y: event.y,
                previous_move: None,
                latest_move: None,
            });
        }
        pressed.into_iter().map(Gesture::Press).collect()
    }

    pub(super) fn pointer_move(
        &mut self,
        route: &mut PointerRoute,
        event: PointerEvent,
    ) -> Vec<Gesture> {
        if route.drag.as_ref().is_some_and(|drag| drag.locked) {
            return self.drag_by(route, event);
        }
        let Some(drag) = route.drag.as_ref() else {
            return self.update_hover(event.x, event.y);
        };
        let past = past_slop(drag.origin_x, drag.origin_y, event.x, event.y);
        if !past {
            return self.move_pressed(route, event);
        }
        let dx = event.x - drag.origin_x;
        let dy = event.y - drag.origin_y;
        let hit = drag.hit;
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
        self.fling.remove(&scroll);
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
        if route.drag.as_ref().is_some_and(|drag| drag.locked) {
            let scroll = route.drag.as_ref().and_then(|drag| drag.scroll);
            let speed = route.drag.as_ref().map(drag_velocity).unwrap_or((0.0, 0.0));
            route.drag = None;
            route.pressed = None;
            self.unlock_pointer(event.pointer);
            return scroll
                .map(|id| {
                    if self.get(id).is_some_and(|node| node.kind == Kind::Scroll) {
                        let _ = self.arm_fling(id, speed.0, speed.1);
                    }
                    vec![Gesture::DragEnd(id, speed.0, speed.1)]
                })
                .unwrap_or_default();
        }
        let hit = route.drag.as_ref().and_then(|drag| drag.hit);
        route.drag = None;
        let Some(target) = route.pressed.take() else {
            return Vec::new();
        };
        let inside = self.point_hits_node(target, event.x, event.y);
        if !inside {
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
        let links = self.get(id)?.text.as_ref()?.links.clone();
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
            Phase::Cancel | Phase::Wheel => Vec::new(),
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
        }
        route.drag = None;
        self.unlock_pointer(pointer);
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
        self.scroll_for(hit, -event.dx, -event.dy, None, false)
            .map(|id| {
                self.fling.remove(&id);
                vec![Gesture::Wheel(id, event.dx, event.dy)]
            })
            .unwrap_or_default()
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
        if node.focusable && node.w > 0.0 && node.h > 0.0 {
            out.push(id);
        }
        if node.kind == Kind::Clip && (node.w == 0.0 || node.h == 0.0) {
            return;
        }
        for child in &node.children {
            self.collect_focusable(*child, out);
        }
    }

    fn geometric_focus(
        &self,
        nodes: &[NodeId],
        current: NodeId,
        direction: FocusDirection,
    ) -> Option<NodeId> {
        let current_rect = self.rect(current).ok()?;
        let cx = current_rect.x + current_rect.w * 0.5;
        let cy = current_rect.y + current_rect.h * 0.5;
        let mut best: Option<(f32, f32, NodeId)> = None;
        for id in nodes {
            if *id == current {
                continue;
            }
            let Ok(rect) = self.rect(*id) else {
                continue;
            };
            let x = rect.x + rect.w * 0.5;
            let y = rect.y + rect.h * 0.5;
            let dx = x - cx;
            let dy = y - cy;
            let primary = match direction {
                FocusDirection::Left => -dx,
                FocusDirection::Right => dx,
                FocusDirection::Up => -dy,
                FocusDirection::Down => dy,
                FocusDirection::Next | FocusDirection::Previous => continue,
            };
            if primary <= 1.0 {
                continue;
            }
            let secondary = match direction {
                FocusDirection::Left | FocusDirection::Right => dy.abs(),
                _ => dx.abs(),
            };
            let replace = match best {
                None => true,
                Some((best_primary, best_secondary, _)) => {
                    primary < best_primary - 0.5
                        || ((primary - best_primary).abs() <= 0.5 && secondary < best_secondary)
                }
            };
            if replace {
                best = Some((primary, secondary, *id));
            }
        }
        best.map(|(_, _, id)| id)
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
    drag.last_x = event.x;
    drag.last_y = event.y;
    (dx, dy)
}

fn drag_velocity(drag: &DragState) -> (f32, f32) {
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
