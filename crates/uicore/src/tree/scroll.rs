//! Scroll offset, overscroll, fling, and snap.

use super::layout::{finite_number, nearest_offset, rubber_band};
use super::node::Kind;
use super::{Gesture, NodeId, Tree};
use crate::Error;
use crate::motion::{Band, OVERSCROLL_FACTOR};

/// Overscroll for a scroll that never had physics installed: the historical
/// linear band.
fn resting_spring(from: f32, to: f32, stiffness: f32, damping: f32) -> crate::Spring {
    crate::Spring {
        x0: from,
        v0: 0.0,
        target: to,
        stiffness,
        damping,
    }
}

fn default_stretch(over: f32, _dimension: f32) -> f32 {
    Band::Linear {
        factor: OVERSCROLL_FACTOR,
    }
    .stretch(over, 0.0)
}

impl Tree {
    pub fn overscroll(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.overscroll())
    }

    /// Scroll paint offset in scene points. Does not run Taffy.
    ///
    /// Hit testing and `record` apply the offset. A later `layout` keeps the
    /// cached boxes when this was the only change.
    pub fn set_scroll_offset(&mut self, id: NodeId, dx: f32, dy: f32) -> Result<(), Error> {
        finite_number(dx)?;
        finite_number(dy)?;
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        let axis = self.axis_of(id);
        let (dx, dy) = axis.filter(dx, dy);
        let (max_x, max_y) = self.scroll_limits(id);
        let (ox, oy) = rubber_band(dx, dy, max_x, max_y);
        let applying = self.applying_motion;
        // The band stretches against the scroll's own viewport: the wider the
        // scroll, the more the same overflow may pull.
        let (view_w, view_h) = {
            let node = self.node(id)?;
            (node.w, node.h)
        };
        let physics = self.scroll_physics(id);
        let stretch = |over: f32, dimension: f32| -> f32 {
            match physics {
                Some(physics) => physics.stretch(over, dimension),
                None => default_stretch(over, dimension),
            }
        };
        let overscroll =
            (!applying).then_some((stretch(dx - ox, view_w), stretch(dy - oy, view_h)));
        let overscroll = overscroll.map(|(sx, sy)| axis.filter(sx, sy));
        {
            let node = self.node_mut(id)?;
            let Some(geom) = node.scroll_mut() else {
                return Err(Error::WrongKind);
            };
            let same_over = match overscroll {
                None => true,
                Some((sx, sy)) => geom.overscroll_x == sx && geom.overscroll_y == sy,
            };
            if geom.offset_x == ox && geom.offset_y == oy && same_over {
                return Ok(());
            }
            geom.offset_x = ox;
            geom.offset_y = oy;
            if let Some((sx, sy)) = overscroll {
                geom.overscroll_x = sx;
                geom.overscroll_y = sy;
            }
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
        self.scroll_semantics.push(id);
        if !self.applying_motion {
            self.rebase_scroll(id, dx, dy);
        }
        Ok(())
    }

    pub fn fling_active(&self) -> bool {
        self.motions.iter().any(|motion| {
            matches!(
                motion.prop,
                crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
            )
        })
    }

    /// Starts coasting. Speeds are scene points per millisecond.
    ///
    /// The finger velocity is negated: a positive drag speed decreases the offset,
    /// matching the previous step. A slow release stops.
    pub fn arm_fling(&mut self, id: NodeId, vx: f32, vy: f32) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        let axis = self.axis_of(id);
        self.motions.retain(|motion| {
            motion.node != id
                || !matches!(
                    motion.prop,
                    crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
                )
        });
        if axis.allows_x() {
            self.arm_scroll_axis(id, crate::AnimProp::ScrollX, vx);
        }
        if axis.allows_y() {
            self.arm_scroll_axis(id, crate::AnimProp::ScrollY, vy);
        }
        self.sync_scroll_phase(id);
        Ok(())
    }

    /// Locks `id` to `axis`. The other component's offset, overscroll, and motions go to zero.
    pub fn set_scroll_axis(&mut self, id: NodeId, axis: crate::ScrollAxis) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        if self.axis_of(id) == axis {
            return Ok(());
        }
        let Some(geom) = self.node_mut(id)?.scroll_mut() else {
            return Err(Error::WrongKind);
        };
        geom.axis = axis;
        self.motions
            .retain(|motion| motion.node != id || axis.allows(motion.prop));
        let (x, y) = self.scroll_offset(id)?;
        self.set_scroll_offset(id, x, y)
    }

    pub fn scroll_axis(&self, id: NodeId) -> Result<crate::ScrollAxis, Error> {
        let node = self.node(id)?;
        node.scroll_geom()
            .map(|geom| geom.axis)
            .ok_or(Error::WrongKind)
    }

    /// Stores physics on `id`. Unknown nodes and non-scrolls fail.
    pub fn set_scroll_physics(
        &mut self,
        id: NodeId,
        physics: crate::ScrollPhysics,
    ) -> Result<(), Error> {
        let Some(geom) = self.node_mut(id)?.scroll_mut() else {
            return Err(Error::WrongKind);
        };
        geom.physics = physics;
        Ok(())
    }

    /// `false` keeps a coast that hits the edge on this scroll.
    /// The default is `true`.
    pub fn set_scroll_chain(&mut self, id: NodeId, chain: bool) -> Result<(), Error> {
        let Some(geom) = self.node_mut(id)?.scroll_mut() else {
            return Err(Error::WrongKind);
        };
        geom.chain = chain;
        Ok(())
    }

    /// Stops coast, snap, and overscroll springs on `id` only.
    pub fn stop_scroll(&mut self, id: NodeId) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        self.motions
            .retain(|motion| motion.node != id || !is_scroll_motion(motion.prop));
        self.note_scroll_phase(id, crate::ScrollPhase::Idle, 0.0, 0.0);
        Ok(())
    }

    /// Springs the offset to `(x, y)` using this scroll's spring, or the given pair when set.
    pub fn scroll_to(
        &mut self,
        id: NodeId,
        x: f32,
        y: f32,
        stiffness: Option<f32>,
        damping: Option<f32>,
    ) -> Result<(), Error> {
        finite_number(x)?;
        finite_number(y)?;
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        let (max_x, max_y) = self.scroll_limits(id);
        let (ox, oy) = self.scroll_offset(id)?;
        let axis = self.axis_of(id);
        let x = if axis.allows_x() {
            x.clamp(0.0, max_x.max(0.0))
        } else {
            ox
        };
        let y = if axis.allows_y() {
            y.clamp(0.0, max_y.max(0.0))
        } else {
            oy
        };
        let physics = self.scroll_physics(id).unwrap_or_default();
        let stiffness = stiffness.unwrap_or(physics.snap_stiffness);
        let damping = damping.unwrap_or(physics.snap_damping);
        if (ox - x).abs() < f32::EPSILON && (oy - y).abs() < f32::EPSILON {
            return Ok(());
        }
        if !(stiffness.is_finite() && stiffness > 0.0) {
            self.set_scroll_offset(id, x, y)?;
            return Ok(());
        }
        self.spring_axis(
            id,
            crate::AnimProp::ScrollX,
            resting_spring(ox, x, stiffness, damping),
            false,
        );
        self.spring_axis(
            id,
            crate::AnimProp::ScrollY,
            resting_spring(oy, y, stiffness, damping),
            false,
        );
        self.note_scroll_phase(id, crate::ScrollPhase::Settling, 0.0, 0.0);
        Ok(())
    }

    /// Friction asymptote clamped to the scroll limits. `None` when this scroll
    /// is not coasting. Callers treat the pair as an upper bound on the offset.
    pub fn fling_target(&self, id: NodeId) -> Option<(f32, f32)> {
        if self.node(id).ok()?.kind != Kind::Scroll {
            return None;
        }
        let (mut x, mut y) = self.scroll_offset(id).ok()?;
        let mut coasting = false;
        for motion in &self.motions {
            if motion.node != id {
                continue;
            }
            match motion.prop {
                crate::AnimProp::ScrollX => {
                    x = motion.simulation.terminal();
                    coasting = true;
                }
                crate::AnimProp::ScrollY => {
                    y = motion.simulation.terminal();
                    coasting = true;
                }
                _ => {}
            }
        }
        if !coasting {
            return None;
        }
        let (max_x, max_y) = self.scroll_limits(id);
        Some((x.clamp(0.0, max_x.max(0.0)), y.clamp(0.0, max_y.max(0.0))))
    }

    /// Springs the nearest scroll ancestors so `id` sits on `edge`.
    /// `Ok(false)` when nothing in the parent chain can scroll.
    pub fn scroll_into_view(&mut self, id: NodeId, edge: super::ScrollEdge) -> Result<bool, Error> {
        self.node(id)?;
        let mut scrolls = Vec::new();
        let mut parent = self.node(id)?.parent;
        while let Some(current) = parent {
            if self.node(current)?.kind == Kind::Scroll {
                scrolls.push(current);
            }
            parent = self.node(current)?.parent;
        }
        if scrolls.is_empty() {
            return Ok(false);
        }
        for scroll in scrolls {
            self.reveal(id, scroll, edge)?;
        }
        Ok(true)
    }

    pub(super) fn reveal(
        &mut self,
        id: NodeId,
        scroll: NodeId,
        edge: super::ScrollEdge,
    ) -> Result<(), Error> {
        let target = self.rect(id)?;
        let view = self.rect(scroll)?;
        let (ox, oy) = self.scroll_offset(scroll)?;
        let (max_x, max_y) = self.scroll_limits(scroll);
        let rel_x = target.x - view.x + ox;
        let rel_y = target.y - view.y + oy;
        let (nx, ny) = match edge {
            super::ScrollEdge::Start => (rel_x, rel_y),
            super::ScrollEdge::End => (rel_x + target.w - view.w, rel_y + target.h - view.h),
            super::ScrollEdge::Nearest => (
                nearest_offset(ox, rel_x, rel_x + target.w, view.w),
                nearest_offset(oy, rel_y, rel_y + target.h, view.h),
            ),
        };
        let axis = self.axis_of(scroll);
        let nx = if axis.allows_x() {
            nx.clamp(0.0, max_x.max(0.0))
        } else {
            ox
        };
        let ny = if axis.allows_y() {
            ny.clamp(0.0, max_y.max(0.0))
        } else {
            oy
        };
        self.spring_scroll(scroll, crate::AnimProp::ScrollX, ox, nx);
        self.spring_scroll(scroll, crate::AnimProp::ScrollY, oy, ny);
        Ok(())
    }

    pub(super) fn spring_scroll(&mut self, id: NodeId, prop: crate::AnimProp, from: f32, to: f32) {
        if (from - to).abs() < f32::EPSILON {
            return;
        }
        let physics = self.scroll_physics(id).unwrap_or_default();
        self.spring_axis(
            id,
            prop,
            resting_spring(from, to, physics.snap_stiffness, physics.snap_damping),
            false,
        );
    }

    fn spring_axis(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        spring: crate::Spring,
        snap: bool,
    ) {
        if (spring.x0 - spring.target).abs() < f32::EPSILON {
            return;
        }
        let simulation = Box::new(spring);
        if snap {
            self.animate_snap(id, prop, simulation);
        } else {
            self.animate(id, prop, simulation);
        }
    }

    pub fn clear_fling(&mut self) {
        self.motions.retain(|motion| {
            !matches!(
                motion.prop,
                crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
            )
        });
    }

    /// Advances the clock by `dt_ms` and samples. Tests use this; the session
    /// passes [`Tree::tick_motions`] an absolute timestamp.
    pub fn tick_fling(&mut self, dt_ms: f64) -> bool {
        let dt = if dt_ms.is_finite() && dt_ms > 0.0 {
            dt_ms
        } else {
            16.0
        };
        self.tick_motions(self.clock_ms + dt)
    }

    pub(super) fn cancel_scroll_motion(&mut self, id: NodeId) {
        self.motions.retain(|motion| {
            motion.node != id
                || !matches!(
                    motion.prop,
                    crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
                )
        });
    }

    /// Adds wheel speed into a coast already running on `id`.
    pub(super) fn merge_wheel(&mut self, id: NodeId, dx: f32, dy: f32) {
        let (dx, dy) = self.axis_of(id).filter(dx, dy);
        self.kick_scroll(id, crate::AnimProp::ScrollX, dx);
        self.kick_scroll(id, crate::AnimProp::ScrollY, dy);
    }

    pub(super) fn kick_scroll(&mut self, id: NodeId, prop: crate::AnimProp, delta: f32) {
        if delta == 0.0 {
            return;
        }
        let Some(index) = self
            .motions
            .iter()
            .position(|motion| motion.node == id && motion.prop == prop)
        else {
            return;
        };
        let t = (self.clock_ms - self.motions[index].started_ms).max(0.0);
        let velocity = self.motions[index].simulation.dx(t) + delta;
        let Ok((x, y)) = self.scroll_offset(id) else {
            return;
        };
        let x0 = if prop == crate::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.motions[index].started_ms = self.clock_ms;
        let tau = self.scroll_tau(id);
        self.motions[index].simulation = Box::new(crate::Friction {
            x0,
            v0: velocity,
            tau_ms: tau,
        });
    }

    pub(super) fn rebase_scroll(&mut self, id: NodeId, x: f32, y: f32) {
        let clock = self.clock_ms;
        let tau = self.scroll_tau(id);
        for motion in &mut self.motions {
            if motion.node != id {
                continue;
            }
            let x0 = match motion.prop {
                crate::AnimProp::ScrollX => x,
                crate::AnimProp::ScrollY => y,
                _ => continue,
            };
            let t = (clock - motion.started_ms).max(0.0);
            let velocity = motion.simulation.dx(t);
            motion.started_ms = clock;
            motion.simulation = Box::new(crate::Friction {
                x0,
                v0: velocity,
                tau_ms: tau,
            });
        }
    }

    pub(super) fn apply_scroll(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
        if !self.axis_of(id).allows(prop) {
            return true;
        }
        let Ok((x, y)) = self.scroll_offset(id) else {
            return true;
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let (nx, ny, hit) = if prop == crate::AnimProp::ScrollX {
            let limit = max_x.max(0.0);
            let next = value.clamp(0.0, limit);
            (next, y, value < -0.5 || value > limit + 0.5)
        } else {
            let limit = max_y.max(0.0);
            let next = value.clamp(0.0, limit);
            (x, next, value < -0.5 || value > limit + 0.5)
        };
        if ((nx - x).abs() > f32::EPSILON || (ny - y).abs() > f32::EPSILON)
            && self.set_scroll_offset(id, nx, ny).is_ok()
        {
            self.scroll_notices.push(Gesture::Scroll(id, nx, ny));
        }
        hit
    }

    pub(super) fn apply_overscroll(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        value: f32,
    ) -> bool {
        if !self.axis_of(id).allows(prop) {
            return true;
        }
        {
            let Ok(node) = self.node_mut(id) else {
                return true;
            };
            let Some(geom) = node.scroll_mut() else {
                return true;
            };
            if prop == crate::AnimProp::OverscrollX {
                geom.overscroll_x = value;
            } else {
                geom.overscroll_y = value;
            }
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
        value.abs() < 0.5
    }

    pub(super) fn arm_overscroll_spring(&mut self, id: NodeId) {
        let Ok((x, y)) = self.node(id).map(|node| node.overscroll()) else {
            return;
        };
        let axis = self.axis_of(id);
        let physics = self.scroll_physics(id).unwrap_or_default();
        if axis.allows_x() && x.abs() > f32::EPSILON {
            self.animate(
                id,
                crate::AnimProp::OverscrollX,
                Box::new(crate::Spring {
                    x0: x,
                    v0: 0.0,
                    target: 0.0,
                    stiffness: physics.overscroll_stiffness,
                    damping: physics.overscroll_damping,
                }),
            );
        }
        if axis.allows_y() && y.abs() > f32::EPSILON {
            self.animate(
                id,
                crate::AnimProp::OverscrollY,
                Box::new(crate::Spring {
                    x0: y,
                    v0: 0.0,
                    target: 0.0,
                    stiffness: physics.overscroll_stiffness,
                    damping: physics.overscroll_damping,
                }),
            );
        }
    }

    pub fn take_scroll_notices(&mut self) -> Vec<Gesture> {
        std::mem::take(&mut self.scroll_notices)
    }

    pub(super) fn scroll_limits(&self, id: NodeId) -> (f32, f32) {
        let Ok(node) = self.node(id) else {
            return (0.0, 0.0);
        };
        let (width, height) = (node.w, node.h);
        let mut child = node.first_child;
        let mut content_w: f32 = 0.0;
        let mut content_h: f32 = 0.0;
        while let Some(current) = child {
            let next = self.node(current).ok().and_then(|node| node.next);
            if let Ok(child) = self.node(current) {
                content_w = content_w.max(child.w);
                content_h = content_h.max(child.h);
            }
            child = next;
        }
        self.axis_of(id)
            .filter((content_w - width).max(0.0), (content_h - height).max(0.0))
    }

    pub(super) fn axis_of(&self, id: NodeId) -> crate::ScrollAxis {
        self.node(id)
            .ok()
            .and_then(|node| node.scroll_geom())
            .map(|geom| geom.axis)
            .unwrap_or(crate::ScrollAxis::Both)
    }

    pub fn scroll_offset(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.scroll_offset())
    }

    /// Coast, drag, or settle. The pull machine reads this after layout.
    pub fn scroll_phase(&self, id: NodeId) -> Result<crate::ScrollPhase, Error> {
        let node = self.node(id)?;
        node.scroll_geom()
            .map(|geom| geom.phase)
            .ok_or(Error::WrongKind)
    }

    fn scroll_physics(&self, id: NodeId) -> Option<crate::ScrollPhysics> {
        self.node(id).ok()?.scroll_geom().map(|geom| geom.physics)
    }

    fn scroll_tau(&self, id: NodeId) -> f32 {
        self.scroll_physics(id)
            .map(|physics| physics.tau_ms())
            .filter(|tau| tau.is_finite() && *tau > 0.0)
            .unwrap_or(crate::SCROLL_TAU_MS)
    }

    fn chain_enabled(&self, id: NodeId) -> bool {
        self.node(id)
            .ok()
            .and_then(|node| node.scroll_geom())
            .is_none_or(|geom| geom.chain)
    }

    fn arm_scroll_axis(&mut self, id: NodeId, prop: crate::AnimProp, finger_v: f32) {
        if finger_v.abs() < 0.02 {
            return;
        }
        let sim_v = -finger_v;
        let owner = self.fling_owner(id, prop, sim_v);
        let Ok((x, y)) = self.scroll_offset(owner) else {
            return;
        };
        let x0 = if prop == crate::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.animate(
            owner,
            prop,
            Box::new(crate::Friction {
                x0,
                v0: sim_v,
                tau_ms: self.scroll_tau(owner),
            }),
        );
    }

    /// The scroll that should coast. Walks ancestors when `id` is already at the edge.
    fn fling_owner(&self, id: NodeId, prop: crate::AnimProp, sim_v: f32) -> NodeId {
        let mut current = id;
        loop {
            if self.can_take_velocity(current, prop, sim_v) || !self.chain_enabled(current) {
                return if self.can_take_velocity(current, prop, sim_v) {
                    current
                } else {
                    id
                };
            }
            let Some(parent) = self.scroll_parent(current) else {
                return id;
            };
            current = parent;
        }
    }

    fn scroll_parent(&self, id: NodeId) -> Option<NodeId> {
        let mut parent = self.node(id).ok()?.parent;
        while let Some(current) = parent {
            if self.node(current).ok()?.kind == Kind::Scroll {
                return Some(current);
            }
            parent = self.node(current).ok()?.parent;
        }
        None
    }

    /// `sim_v` is the offset derivative. Positive moves toward the far edge.
    fn can_take_velocity(&self, id: NodeId, prop: crate::AnimProp, sim_v: f32) -> bool {
        if !self.axis_of(id).allows(prop) || sim_v.abs() < 0.02 {
            return false;
        }
        let Ok(node) = self.node(id) else {
            return false;
        };
        if node.kind != Kind::Scroll {
            return false;
        }
        let (offset_x, offset_y) = node.scroll_offset();
        let (max_x, max_y) = self.scroll_limits(id);
        let (offset, limit) = if prop == crate::AnimProp::ScrollX {
            (offset_x, max_x.max(0.0))
        } else {
            (offset_y, max_y.max(0.0))
        };
        if limit <= 0.5 {
            return false;
        }
        if sim_v > 0.0 {
            offset < limit - 0.5
        } else {
            offset > 0.5
        }
    }

    pub(super) fn after_scroll_motion(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        velocity: f32,
        snap: bool,
    ) {
        if !snap
            && velocity.abs() >= 0.02
            && self.axis_blocked(id, prop, velocity)
            && self.handoff_fling(id, prop, velocity)
        {
            self.note_scroll_phase(id, crate::ScrollPhase::Idle, 0.0, 0.0);
            return;
        }
        if !snap && let Some(target) = self.snap_target(id, prop) {
            let current = self.axis_offset(id, prop);
            if (target - current).abs() > 0.5 {
                let physics = self.scroll_physics(id).unwrap_or_default();
                self.spring_axis(
                    id,
                    prop,
                    resting_spring(
                        current,
                        target,
                        physics.snap_stiffness,
                        physics.snap_damping,
                    ),
                    true,
                );
                self.note_scroll_phase(id, crate::ScrollPhase::Settling, 0.0, 0.0);
                return;
            }
        }
        self.sync_scroll_phase(id);
    }

    fn handoff_fling(&mut self, id: NodeId, prop: crate::AnimProp, sim_v: f32) -> bool {
        if !self.chain_enabled(id) {
            return false;
        }
        let Some(parent) = self.scroll_parent(id) else {
            return false;
        };
        let owner = self.fling_owner(parent, prop, sim_v);
        if !self.can_take_velocity(owner, prop, sim_v) {
            return false;
        }
        let Ok((x, y)) = self.scroll_offset(owner) else {
            return false;
        };
        let x0 = if prop == crate::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.animate(
            owner,
            prop,
            Box::new(crate::Friction {
                x0,
                v0: sim_v,
                tau_ms: self.scroll_tau(owner),
            }),
        );
        let (vx, vy) = if prop == crate::AnimProp::ScrollX {
            (sim_v, 0.0)
        } else {
            (0.0, sim_v)
        };
        self.note_scroll_phase(owner, crate::ScrollPhase::Coasting, vx, vy);
        true
    }

    fn axis_blocked(&self, id: NodeId, prop: crate::AnimProp, sim_v: f32) -> bool {
        !self.can_take_velocity(id, prop, sim_v)
    }

    fn axis_offset(&self, id: NodeId, prop: crate::AnimProp) -> f32 {
        let Ok((x, y)) = self.scroll_offset(id) else {
            return 0.0;
        };
        if prop == crate::AnimProp::ScrollX {
            x
        } else {
            y
        }
    }

    fn snap_target(&self, id: NodeId, prop: crate::AnimProp) -> Option<f32> {
        if !self.axis_of(id).allows(prop) {
            return None;
        }
        let physics = self.scroll_physics(id)?;
        let grid = match prop {
            crate::AnimProp::ScrollX => physics.snap_x,
            crate::AnimProp::ScrollY => physics.snap_y,
            _ => None,
        }?;
        if !(grid.interval.is_finite() && grid.interval > 0.5) {
            return None;
        }
        let (offset_x, offset_y) = self.scroll_offset(id).ok()?;
        let node = self.node(id).ok()?;
        let (offset, view) = if prop == crate::AnimProp::ScrollX {
            (offset_x, node.w)
        } else {
            (offset_y, node.h)
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let limit = if prop == crate::AnimProp::ScrollX {
            max_x.max(0.0)
        } else {
            max_y.max(0.0)
        };
        let origin = match grid.align {
            crate::SnapAlign::Start => offset,
            crate::SnapAlign::Center => offset + view * 0.5,
            crate::SnapAlign::End => offset + view,
        };
        let index = (origin / grid.interval).round();
        let snapped = index * grid.interval;
        let next = match grid.align {
            crate::SnapAlign::Start => snapped,
            crate::SnapAlign::Center => snapped - view * 0.5,
            crate::SnapAlign::End => snapped - view,
        };
        Some(next.clamp(0.0, limit))
    }

    pub(super) fn sync_scroll_phase(&mut self, id: NodeId) {
        let coasting = self.motions.iter().any(|motion| {
            motion.node == id
                && matches!(
                    motion.prop,
                    crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
                )
                && !motion.snap
        });
        let settling = self.motions.iter().any(|motion| {
            motion.node == id
                && (motion.snap
                    || matches!(
                        motion.prop,
                        crate::AnimProp::OverscrollX | crate::AnimProp::OverscrollY
                    ))
        });
        let phase = if coasting {
            crate::ScrollPhase::Coasting
        } else if settling {
            crate::ScrollPhase::Settling
        } else {
            crate::ScrollPhase::Idle
        };
        self.note_scroll_phase(id, phase, 0.0, 0.0);
    }

    pub(super) fn note_scroll_phase(
        &mut self,
        id: NodeId,
        phase: crate::ScrollPhase,
        vx: f32,
        vy: f32,
    ) {
        let (ox, oy, changed) = {
            let Ok(node) = self.node_mut(id) else {
                return;
            };
            let Some(geom) = node.scroll_mut() else {
                return;
            };
            if geom.phase == phase {
                return;
            }
            geom.phase = phase;
            (geom.overscroll_x, geom.overscroll_y, true)
        };
        if changed {
            self.scroll_notices.push(Gesture::ScrollPhaseChanged {
                id,
                phase,
                vx,
                vy,
                ox,
                oy,
            });
        }
    }
}

fn is_scroll_motion(prop: crate::AnimProp) -> bool {
    matches!(
        prop,
        crate::AnimProp::ScrollX
            | crate::AnimProp::ScrollY
            | crate::AnimProp::OverscrollX
            | crate::AnimProp::OverscrollY
    )
}

impl Tree {
    /// Snap a scroll's offsets to whole device pixels after its motion ends.
    /// A no-op at the default pixel scale of one.
    pub(super) fn settle_scroll_pixels(&mut self, id: NodeId) {
        if (self.pixel_scale - 1.0).abs() <= f32::EPSILON {
            return;
        }
        let Ok((x, y)) = self.scroll_offset(id) else {
            return;
        };
        let snapped = (self.settle_pixels(x), self.settle_pixels(y));
        if (snapped.0 - x).abs() > f32::EPSILON || (snapped.1 - y).abs() > f32::EPSILON {
            let _ = self.set_scroll_offset(id, snapped.0, snapped.1);
        }
    }
}
