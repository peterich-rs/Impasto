//! Press, drag, focus, hover, wheel, and the right button.
//!
//! The right button keeps its own press. It never writes [`Tree::pressed`] or [`Tree::drag`].

use super::{Gesture, NodeId, Tree};
use crate::{Button, Error, Phase, PointerEvent};

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
    pub(super) scroll: Option<NodeId>,
    pub(super) locked: bool,
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) last_x: f32,
    pub(super) last_y: f32,
    pub(super) previous_move: Option<Sample>,
    pub(super) latest_move: Option<Sample>,
}

/// Right button press. Independent of [`Tree::pressed`] and [`Tree::drag`].
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

    /// Moves focus to `id` when it is marked focusable. The returned transition
    /// is empty when `id` is already focused.
    pub fn focus(&mut self, id: NodeId) -> Result<Vec<Gesture>, Error> {
        if !self.node(id)?.focusable {
            return Err(Error::NotFocusable);
        }
        Ok(self.move_focus(Some(id)))
    }

    pub fn blur(&mut self) -> Vec<Gesture> {
        self.move_focus(None)
    }

    /// Hit-tests the previous layout and updates the press, drag, and focus gestures.
    ///
    /// A hit on the way down is [`Gesture::Press`]. A move that stays within
    /// [`DRAG_SLOP`] keeps that press. Past the slop, the nearest [`Kind::Scroll`]
    /// ancestor locks the pointer: the press is released and later moves are
    /// [`Gesture::DragBy`]. A lift inside the pressed region, with no lock, is
    /// [`Gesture::Release`] then [`Gesture::Click`], and that click is what moves
    /// focus. A locked drag does not. Does not lay out.
    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let mut gestures = self.drain_pending();
        if !self.laid_out {
            return gestures;
        }
        gestures.extend(match event.phase {
            Phase::Wheel => self.pointer_wheel(event),
            Phase::Cancel => self.pointer_cancel(),
            Phase::Down | Phase::Move | Phase::Up if event.button == Button::Secondary => {
                self.pointer_secondary(event)
            }
            Phase::Down | Phase::Up if event.button != Button::Primary => Vec::new(),
            Phase::Move if event.button != Button::None && event.button != Button::Primary => {
                Vec::new()
            }
            Phase::Down => self.pointer_down(event),
            Phase::Move => self.pointer_move(event),
            Phase::Up => self.pointer_up(event),
        });
        gestures
    }

    /// Primary press only. Callers keep other buttons out of this path.
    pub(super) fn pointer_down(&mut self, event: PointerEvent) -> Vec<Gesture> {
        self.drag = None;
        self.pressed = None;
        let hit = self.hit_root(event.x, event.y);
        let pressed = hit.and_then(|id| self.region_of(id));
        let scroll = hit.and_then(|id| self.drag_of(id));
        self.pressed = pressed;
        if pressed.is_some() || scroll.is_some() {
            self.drag = Some(DragState {
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

    pub(super) fn pointer_move(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if self.drag.as_ref().is_some_and(|drag| drag.locked) {
            return self.drag_by(event);
        }
        let Some(drag) = self.drag.as_ref() else {
            return self.update_hover(event.x, event.y);
        };
        let past = past_slop(drag.origin_x, drag.origin_y, event.x, event.y);
        let scroll = drag.scroll.filter(|_| past);
        if let Some(scroll) = scroll {
            return self.lock_drag(event, scroll);
        }
        self.move_pressed(event)
    }

    pub(super) fn drag_by(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let Some(scroll) = drag.scroll else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        vec![Gesture::DragBy(scroll, dx, dy)]
    }

    pub(super) fn lock_drag(&mut self, event: PointerEvent, scroll: NodeId) -> Vec<Gesture> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        drag.locked = true;
        let mut gestures = Vec::new();
        if let Some(target) = self.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        gestures.push(Gesture::DragBegin(scroll));
        gestures.push(Gesture::DragBy(scroll, dx, dy));
        gestures
    }

    pub(super) fn move_pressed(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if let Some(drag) = self.drag.as_mut() {
            note_move(drag, event);
        }
        let Some(target) = self.pressed else {
            return Vec::new();
        };
        let still = self
            .hit_root(event.x, event.y)
            .is_some_and(|hit| self.contains(target, hit));
        if still {
            Vec::new()
        } else {
            self.pressed = None;
            self.drag = None;
            vec![Gesture::Release(target)]
        }
    }

    pub(super) fn pointer_up(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if self.drag.as_ref().is_some_and(|drag| drag.locked) {
            let scroll = self.drag.as_ref().and_then(|drag| drag.scroll);
            let speed = self.drag.as_ref().map(drag_velocity).unwrap_or((0.0, 0.0));
            self.drag = None;
            self.pressed = None;
            return scroll
                .map(|id| vec![Gesture::DragEnd(id, speed.0, speed.1)])
                .unwrap_or_default();
        }
        self.drag = None;
        let Some(target) = self.pressed.take() else {
            return Vec::new();
        };
        let hit = self.hit_root(event.x, event.y);
        let inside = hit.is_some_and(|hit| self.contains(target, hit));
        if !inside {
            return vec![Gesture::Release(target)];
        }
        let mut gestures = vec![Gesture::Release(target), Gesture::Click(target)];
        let next = hit.and_then(|id| self.focusable_of(id));
        gestures.extend(self.move_focus(next));
        gestures
    }

    pub(super) fn pointer_secondary(&mut self, event: PointerEvent) -> Vec<Gesture> {
        match event.phase {
            Phase::Down => {
                let target = self
                    .hit_root(event.x, event.y)
                    .and_then(|id| self.region_of(id));
                self.secondary = target.map(|target| SecondaryPress {
                    target,
                    origin_x: event.x,
                    origin_y: event.y,
                    cancelled: false,
                });
                Vec::new()
            }
            Phase::Move => {
                if let Some(press) = self.secondary.as_mut()
                    && !press.cancelled
                    && past_slop(press.origin_x, press.origin_y, event.x, event.y)
                {
                    press.cancelled = true;
                }
                Vec::new()
            }
            Phase::Up => {
                let Some(press) = self.secondary.take() else {
                    return Vec::new();
                };
                if press.cancelled {
                    return Vec::new();
                }
                let hit = self.hit_root(event.x, event.y);
                if hit.is_some_and(|hit| self.contains(press.target, hit)) {
                    vec![Gesture::SecondaryClick(press.target)]
                } else {
                    Vec::new()
                }
            }
            Phase::Cancel | Phase::Wheel => Vec::new(),
        }
    }

    pub(super) fn pointer_cancel(&mut self) -> Vec<Gesture> {
        self.secondary = None;
        let mut gestures = Vec::new();
        if let Some(target) = self.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        if self.drag.as_ref().is_some_and(|drag| drag.locked)
            && let Some(scroll) = self.drag.as_ref().and_then(|drag| drag.scroll)
        {
            gestures.push(Gesture::DragEnd(scroll, 0.0, 0.0));
        }
        self.drag = None;
        gestures
    }

    pub(super) fn pointer_wheel(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(hit) = self.hit_root(event.x, event.y) else {
            return Vec::new();
        };
        self.drag_of(hit)
            .map(|id| vec![Gesture::Wheel(id, event.dx, event.dy)])
            .unwrap_or_default()
    }

    pub(super) fn update_hover(&mut self, x: f32, y: f32) -> Vec<Gesture> {
        let next = self.hit_root(x, y);
        if self.hovered == next {
            return Vec::new();
        }
        let mut gestures = Vec::new();
        if let Some(old) = self.hovered.take() {
            gestures.push(Gesture::Leave(old));
        }
        if let Some(id) = next {
            self.hovered = Some(id);
            gestures.push(Gesture::Hover(id));
        }
        gestures
    }

    pub(super) fn drain_pending(&mut self) -> Vec<Gesture> {
        std::mem::take(&mut self.pending)
    }

    pub(super) fn move_focus(&mut self, next: Option<NodeId>) -> Vec<Gesture> {
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
