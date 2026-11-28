//! Scroll offset, overscroll, fling, and snap.

use super::layout::{finite_number, nearest_offset, rubber_band};
use super::node::Kind;
use super::{Gesture, NodeId, Tree};
use crate::Error;
use crate::motion::Band;

/// Offset and overscroll after [`Tree::resolve_scroll`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollSample {
    pub offset_x: f32,
    pub offset_y: f32,
    pub overscroll_x: f32,
    pub overscroll_y: f32,
}

fn resting_spring(from: f32, to: f32, stiffness: f32, damping: f32) -> crate::motion::Spring {
    crate::motion::Spring {
        x0: from,
        v0: 0.0,
        target: to,
        stiffness,
        damping,
    }
}

impl Tree {
    pub fn take_scroll_paint_updates(&mut self) -> u32 {
        std::mem::take(&mut self.scroll_paint_updates)
    }

    fn mark_scroll_paint(&mut self) {
        self.scroll_paint_updates = self.scroll_paint_updates.saturating_add(1);
        self.canvas_geometry_changed();
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    pub fn overscroll(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.overscroll())
    }

    /// Unclamped desired offset matching current scroll offset plus unstretched overscroll.
    pub(super) fn unclamped_scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.unstretch_scroll(id, self.scroll_physics(id).unwrap_or_default().band)
    }

    fn unstretch_scroll(&self, id: NodeId, band: Band) -> (f32, f32) {
        let Ok(node) = self.node(id) else {
            return (0.0, 0.0);
        };
        let (ox, oy) = node.scroll_offset();
        let (sx, sy) = node.overscroll();
        (
            ox + band.unstretch(sx, node.w),
            oy + band.unstretch(sy, node.h),
        )
    }

    pub(super) fn resolve_wheel_drag(&mut self, id: NodeId, dx: f32, dy: f32) {
        let physics = self.scroll_physics(id).unwrap_or_default().for_trackpad();
        let (x, y) = self.unstretch_scroll(id, physics.band);
        let _ = self.resolve_scroll_with_physics(id, x + dx, y + dy, physics);
    }

    pub(super) fn has_overscroll_motion(&self, id: NodeId) -> bool {
        self.motions.iter().any(|motion| {
            motion.node == id
                && matches!(
                    motion.prop,
                    crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY
                )
        })
    }

    /// Scroll paint offset in scene points. Does not run Taffy.
    ///
    /// Hit testing and `record` apply the offset. A later `layout` keeps the
    /// cached boxes when this was the only change. The value is resolved by
    /// [`Tree::resolve_scroll`]. A locked drag on this node continues from the
    /// requested position, not from the clamped offset.
    pub fn set_scroll_offset(&mut self, id: NodeId, dx: f32, dy: f32) -> Result<(), Error> {
        let _ = self.resolve_scroll(id, dx, dy)?;
        self.rebase_drag_desired(id, dx, dy);
        Ok(())
    }

    /// The only clamp. Drag, wheel, and [`Tree::set_scroll_offset`] all call this.
    ///
    /// Offset stays inside the limits. Overscroll is the stretch of whatever
    /// this call did not clamp away. Motion samples do not stretch again.
    pub fn resolve_scroll(
        &mut self,
        id: NodeId,
        desired_x: f32,
        desired_y: f32,
    ) -> Result<ScrollSample, Error> {
        let physics = self.scroll_physics(id).unwrap_or_default();
        self.resolve_scroll_with_physics(id, desired_x, desired_y, physics)
    }

    fn resolve_scroll_with_physics(
        &mut self,
        id: NodeId,
        desired_x: f32,
        desired_y: f32,
        physics: crate::motion::ScrollPhysics,
    ) -> Result<ScrollSample, Error> {
        finite_number(desired_x)?;
        finite_number(desired_y)?;
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        let axis = self.axis_of(id);
        let (desired_x, desired_y) = axis.filter(desired_x, desired_y);
        let (max_x, max_y) = self.scroll_limits(id);
        let (ox, oy) = rubber_band(desired_x, desired_y, max_x, max_y);
        let applying = self.applying_motion;
        let (view_w, view_h) = {
            let node = self.node(id)?;
            (node.w, node.h)
        };
        let stretch = |over: f32, dimension: f32, limit: f32| -> f32 {
            if physics.accept == crate::motion::ScrollAccept::Overflow && limit <= 0.5 {
                0.0
            } else {
                physics.stretch(over, dimension)
            }
        };
        let overscroll = if applying {
            None
        } else {
            Some(axis.filter(
                stretch(desired_x - ox, view_w, max_x),
                stretch(desired_y - oy, view_h, max_y),
            ))
        };
        let wrote = {
            let node = self.node_mut(id)?;
            let Some(geom) = node.scroll_mut() else {
                return Err(Error::WrongKind);
            };
            let (sx, sy) = overscroll.unwrap_or((geom.overscroll_x, geom.overscroll_y));
            let same = geom.offset_x == ox
                && geom.offset_y == oy
                && geom.overscroll_x == sx
                && geom.overscroll_y == sy;
            if !same {
                geom.offset_x = ox;
                geom.offset_y = oy;
                if overscroll.is_some() {
                    geom.overscroll_x = sx;
                    geom.overscroll_y = sy;
                }
            }
            !same
        };
        if wrote {
            self.mark_scroll_paint();
            if self.publish_semantics() {
                self.scroll_semantics.push(id);
            }
            if !applying {
                self.rebase_scroll(id, desired_x, desired_y);
                self.scroll_notices.push(Gesture::Scroll(id, ox, oy));
            }
        }
        let (sx, sy) = self.node(id)?.overscroll();
        Ok(ScrollSample {
            offset_x: ox,
            offset_y: oy,
            overscroll_x: sx,
            overscroll_y: sy,
        })
    }

    /// Writes a finger's unclamped desired offset. Ancestors on the fling chain
    /// take the remainder. What nobody takes stretches on the end of that chain.
    pub(super) fn apply_drag_desired(
        &mut self,
        id: NodeId,
        desired_x: f32,
        desired_y: f32,
    ) -> Result<ScrollSample, Error> {
        let (ox, oy) = self.scroll_offset(id)?;
        let axis = self.axis_of(id);
        let (desired_x, desired_y) = axis.filter(desired_x, desired_y);
        let (max_x, max_y) = self.scroll_limits(id);
        let (cx, cy) = rubber_band(desired_x, desired_y, max_x, max_y);
        let rem_x = desired_x - cx;
        let rem_y = desired_y - cy;
        let mut kept_x = rem_x;
        let mut kept_y = rem_y;
        if self.chain_enabled(id)
            && let Some(parent) = self.scroll_parent(id)
        {
            self.give_remainder(parent, rem_x, rem_y);
            kept_x = 0.0;
            kept_y = 0.0;
        }
        let _ = (ox, oy);
        self.resolve_scroll(id, cx + kept_x, cy + kept_y)
    }

    /// Parent chain absorbs `rem`. The node that cannot pass it further stretches.
    fn give_remainder(&mut self, id: NodeId, rem_x: f32, rem_y: f32) {
        let Ok((ox, oy)) = self.scroll_offset(id) else {
            return;
        };
        let axis = self.axis_of(id);
        let (pass_x, pass_y) = axis.filter(rem_x, rem_y);
        let (max_x, max_y) = self.scroll_limits(id);
        let (cx, cy) = rubber_band(ox + pass_x, oy + pass_y, max_x, max_y);
        let rest_x = ox + pass_x - cx;
        let rest_y = oy + pass_y - cy;
        let end = !self.chain_enabled(id) || self.scroll_parent(id).is_none();
        if !end && let Some(parent) = self.scroll_parent(id) {
            self.give_remainder(parent, rest_x, rest_y);
            let _ = self.resolve_scroll(id, cx, cy);
            return;
        }
        let _ = self.resolve_scroll(id, cx + rest_x, cy + rest_y);
    }

    /// Floor used by `Tree::scroll_limits`. Does not run layout.
    pub fn set_content_min(&mut self, id: NodeId, min: f32) -> Result<(), Error> {
        finite_number(min)?;
        let Some(geom) = self.node_mut(id)?.scroll_mut() else {
            return Err(Error::WrongKind);
        };
        geom.content_min = min.max(0.0);
        Ok(())
    }

    /// Refresh threshold. `Overscroll` commits at idle. `Edge` commits on drag end.
    pub fn set_scroll_pull(
        &mut self,
        id: NodeId,
        mode: crate::motion::PullMode,
        threshold: f32,
    ) -> Result<(), Error> {
        if !threshold.is_finite() || threshold < 0.0 {
            return Err(Error::BadNumber);
        }
        let Some(geom) = self.node_mut(id)?.scroll_mut() else {
            return Err(Error::WrongKind);
        };
        geom.pull_mode = mode;
        geom.pull_threshold = threshold;
        Ok(())
    }

    pub fn take_refresh(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.refresh_pending)
    }

    /// Edge-mode loss at the top. Called with the unclamped desired and the clamp.
    pub(super) fn note_edge_pull(&mut self, id: NodeId, raw_y: f32, next_y: f32) {
        let Ok(node) = self.node(id) else {
            return;
        };
        let Some(geom) = node.scroll_geom() else {
            return;
        };
        if geom.pull_mode != crate::motion::PullMode::Edge {
            return;
        }
        let live_y = geom.offset_y;
        let delta = crate::tree::list::pull_delta(live_y, raw_y, next_y);
        if delta <= 0.0 {
            return;
        }
        let Some(geom) = self.node_mut(id).ok().and_then(|node| node.scroll_mut()) else {
            return;
        };
        geom.pull += delta;
    }

    pub(super) fn note_overscroll_pull(&mut self, id: NodeId) {
        let Ok((_, oy)) = self.overscroll(id) else {
            return;
        };
        let Some(geom) = self.node_mut(id).ok().and_then(|node| node.scroll_mut()) else {
            return;
        };
        if geom.pull_mode != crate::motion::PullMode::Overscroll {
            return;
        }
        let distance = (-oy).max(0.0);
        if geom.pull_threshold > 0.0 && distance >= geom.pull_threshold {
            geom.pull_armed = true;
        }
    }

    pub(super) fn commit_edge_pull(&mut self, id: NodeId) {
        let fire = {
            let Ok(node) = self.node(id) else {
                return;
            };
            let Some(geom) = node.scroll_geom() else {
                return;
            };
            geom.pull_mode == crate::motion::PullMode::Edge && geom.pull > geom.pull_threshold
        };
        if let Some(geom) = self.node_mut(id).ok().and_then(|node| node.scroll_mut()) {
            geom.pull = 0.0;
        }
        if fire {
            self.refresh_pending.push(id);
        }
    }

    fn commit_overscroll_pull(&mut self, id: NodeId) {
        let fire = {
            let Ok(node) = self.node(id) else {
                return;
            };
            let Some(geom) = node.scroll_geom() else {
                return;
            };
            geom.pull_armed && geom.pull_mode == crate::motion::PullMode::Overscroll
        };
        if let Some(geom) = self.node_mut(id).ok().and_then(|node| node.scroll_mut())
            && geom.pull_mode == crate::motion::PullMode::Overscroll
        {
            geom.pull_armed = false;
        }
        if fire {
            self.refresh_pending.push(id);
        }
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
                    crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY
                )
        });
        if axis.allows_x() {
            self.arm_scroll_axis(id, crate::motion::AnimProp::ScrollX, vx);
        }
        if axis.allows_y() {
            self.arm_scroll_axis(id, crate::motion::AnimProp::ScrollY, vy);
        }
        self.sync_scroll_phase(id);
        Ok(())
    }

    /// Locks `id` to `axis`. The other component's offset, overscroll, and motions go to zero.
    pub fn set_scroll_axis(
        &mut self,
        id: NodeId,
        axis: crate::motion::ScrollAxis,
    ) -> Result<(), Error> {
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
        self.sync_scroll_tracks(id)?;
        let (x, y) = self.scroll_offset(id)?;
        self.set_scroll_offset(id, x, y)
    }

    /// Match the grid tracks to the axis. See [`super::layout::scroll_style_for`].
    fn sync_scroll_tracks(&mut self, id: NodeId) -> Result<(), Error> {
        let next = super::layout::scroll_style_for(self.axis_of(id));
        let mut style = self.node(id)?.style.clone();
        style.grid_template_columns = next.grid_template_columns;
        style.grid_template_rows = next.grid_template_rows;
        style.align_items = next.align_items;
        style.justify_items = next.justify_items;
        self.write_style(id, style)
    }

    pub fn scroll_axis(&self, id: NodeId) -> Result<crate::motion::ScrollAxis, Error> {
        let node = self.node(id)?;
        node.scroll_geom()
            .map(|geom| geom.axis)
            .ok_or(Error::WrongKind)
    }

    /// Stores physics on `id`. Unknown nodes and non-scrolls fail.
    pub fn set_scroll_physics(
        &mut self,
        id: NodeId,
        physics: crate::motion::ScrollPhysics,
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
        self.note_scroll_phase(id, crate::motion::ScrollPhase::Idle, 0.0, 0.0);
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
            crate::motion::AnimProp::ScrollX,
            resting_spring(ox, x, stiffness, damping),
            false,
        );
        self.spring_axis(
            id,
            crate::motion::AnimProp::ScrollY,
            resting_spring(oy, y, stiffness, damping),
            false,
        );
        self.note_scroll_phase(id, crate::motion::ScrollPhase::Settling, 0.0, 0.0);
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
                crate::motion::AnimProp::ScrollX => {
                    x = motion.simulation.terminal();
                    coasting = true;
                }
                crate::motion::AnimProp::ScrollY => {
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
        self.spring_scroll(scroll, crate::motion::AnimProp::ScrollX, ox, nx);
        self.spring_scroll(scroll, crate::motion::AnimProp::ScrollY, oy, ny);
        Ok(())
    }

    pub(super) fn spring_scroll(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        from: f32,
        to: f32,
    ) {
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
        prop: crate::motion::AnimProp,
        spring: crate::motion::Spring,
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
                crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY
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

    /// Drops coast and overscroll springs on `id`. A locked drag owns both.
    pub(super) fn cancel_scroll_motion(&mut self, id: NodeId) {
        self.motions
            .retain(|motion| motion.node != id || !is_scroll_motion(motion.prop));
    }

    /// Adds wheel speed into a coast already running on `id`.
    pub(super) fn merge_wheel(&mut self, id: NodeId, dx: f32, dy: f32) {
        let (dx, dy) = self.axis_of(id).filter(dx, dy);
        self.kick_scroll(id, crate::motion::AnimProp::ScrollX, dx);
        self.kick_scroll(id, crate::motion::AnimProp::ScrollY, dy);
    }

    pub(super) fn kick_scroll(&mut self, id: NodeId, prop: crate::motion::AnimProp, delta: f32) {
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
        let x0 = if prop == crate::motion::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.motions[index].started_ms = self.clock_ms;
        let tau = self.scroll_tau(id);
        self.motions[index].simulation = Box::new(crate::motion::Friction {
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
                crate::motion::AnimProp::ScrollX => x,
                crate::motion::AnimProp::ScrollY => y,
                _ => continue,
            };
            let t = (clock - motion.started_ms).max(0.0);
            let velocity = motion.simulation.dx(t);
            motion.started_ms = clock;
            motion.simulation = Box::new(crate::motion::Friction {
                x0,
                v0: velocity,
                tau_ms: tau,
            });
        }
    }

    pub(super) fn apply_scroll(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        value: f32,
    ) -> bool {
        if !self.axis_of(id).allows(prop) {
            return true;
        }
        let Ok((x, y)) = self.scroll_offset(id) else {
            return true;
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let (nx, ny, hit) = if prop == crate::motion::AnimProp::ScrollX {
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
        prop: crate::motion::AnimProp,
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
            if prop == crate::motion::AnimProp::OverscrollX {
                geom.overscroll_x = value;
            } else {
                geom.overscroll_y = value;
            }
        }
        self.mark_scroll_paint();
        // A coast that hits the wall starts an overscroll spring at x0 = 0
        // with leftover velocity. Stopping at |value| < 0.5 would kill that
        // spring on the first sample. `done` and the sign-cross in
        // `tick_motions` land it.
        false
    }

    /// `finger_vx` and `finger_vy` are scene points per millisecond, the same
    /// sign as pointer movement. A positive finger speed decreases offset, so
    /// the overscroll derivative is the negation times the band slope.
    pub(super) fn arm_overscroll_spring(&mut self, id: NodeId, finger_vx: f32, finger_vy: f32) {
        let physics = self.scroll_physics(id).unwrap_or_default();
        self.arm_scroll_return(id, (-finger_vx, -finger_vy), physics);
    }

    pub(super) fn arm_wheel_return(&mut self, id: NodeId, vx: f32, vy: f32) {
        let physics = self.scroll_physics(id).unwrap_or_default().for_trackpad();
        // Carry a fraction of contact velocity through the release. Bound the
        // offset derivative before mapping it through the local band slope.
        let release_velocity = |v: f32| (v * 0.35).clamp(-2.0, 2.0);
        self.arm_scroll_return(id, (release_velocity(vx), release_velocity(vy)), physics);
    }

    fn arm_scroll_return(
        &mut self,
        id: NodeId,
        (vx, vy): (f32, f32),
        physics: crate::motion::ScrollPhysics,
    ) {
        let Ok(node) = self.node(id) else {
            return;
        };
        let (x, y) = node.overscroll();
        let (view_w, view_h) = (node.w, node.h);
        let axis = self.axis_of(id);
        if axis.allows_x() && x.abs() > f32::EPSILON {
            self.arm_overscroll_axis(
                id,
                crate::motion::AnimProp::OverscrollX,
                x,
                vx * physics.band.slope_at_stretch(x, view_w),
                physics,
            );
        }
        if axis.allows_y() && y.abs() > f32::EPSILON {
            self.arm_overscroll_axis(
                id,
                crate::motion::AnimProp::OverscrollY,
                y,
                vy * physics.band.slope_at_stretch(y, view_h),
                physics,
            );
        }
    }

    fn arm_overscroll_axis(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        x0: f32,
        v0: f32,
        physics: crate::motion::ScrollPhysics,
    ) {
        if physics.overscroll == crate::motion::OverscrollMode::Clamp {
            return;
        }
        if x0.abs() <= f32::EPSILON && v0.abs() < 0.02 {
            return;
        }
        self.animate(
            id,
            prop,
            Box::new(crate::motion::Spring {
                x0,
                v0,
                target: 0.0,
                stiffness: physics.overscroll_stiffness,
                damping: physics.overscroll_damping,
            }),
        );
    }

    pub fn take_scroll_notices(&mut self) -> Vec<Gesture> {
        std::mem::take(&mut self.scroll_notices)
    }

    pub(super) fn scroll_limits(&self, id: NodeId) -> (f32, f32) {
        let Ok(node) = self.node(id) else {
            return (0.0, 0.0);
        };
        let height = node.h;
        let taffy_id = node.taffy;
        let content_min = node
            .scroll_geom()
            .map(|geom| geom.content_min)
            .unwrap_or(0.0);
        let (scroll_x, scroll_y) = taffy_id
            .and_then(|taffy_id| self.taffy.layout(taffy_id).ok())
            .map(|layout| (layout.scroll_width(), layout.scroll_height()))
            .unwrap_or((0.0, 0.0));
        // `scroll_width` / `scroll_height` are already the maximum offsets.
        let limit_y = scroll_y.max((content_min - height).max(0.0));
        self.axis_of(id).filter(scroll_x, limit_y)
    }

    pub(super) fn axis_of(&self, id: NodeId) -> crate::motion::ScrollAxis {
        self.node(id)
            .ok()
            .and_then(|node| node.scroll_geom())
            .map(|geom| geom.axis)
            .unwrap_or(crate::motion::ScrollAxis::Both)
    }

    pub fn scroll_offset(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.scroll_offset())
    }

    /// Coast, drag, or settle. The pull machine reads this after layout.
    pub fn scroll_phase(&self, id: NodeId) -> Result<crate::motion::ScrollPhase, Error> {
        let node = self.node(id)?;
        node.scroll_geom()
            .map(|geom| geom.phase)
            .ok_or(Error::WrongKind)
    }

    fn scroll_physics(&self, id: NodeId) -> Option<crate::motion::ScrollPhysics> {
        self.node(id).ok()?.scroll_geom().map(|geom| geom.physics)
    }

    fn scroll_tau(&self, id: NodeId) -> f32 {
        self.scroll_physics(id)
            .map(|physics| physics.tau_ms())
            .filter(|tau| tau.is_finite() && *tau > 0.0)
            .unwrap_or(crate::motion::SCROLL_TAU_MS)
    }

    fn chain_enabled(&self, id: NodeId) -> bool {
        self.node(id)
            .ok()
            .and_then(|node| node.scroll_geom())
            .is_none_or(|geom| geom.chain)
    }

    fn arm_scroll_axis(&mut self, id: NodeId, prop: crate::motion::AnimProp, finger_v: f32) {
        if finger_v.abs() < 0.02 {
            return;
        }
        let sim_v = -finger_v;
        let owner = self.fling_owner(id, prop, sim_v);
        let Ok((over_x, over_y)) = self.overscroll(owner) else {
            return;
        };
        let overscroll = if prop == crate::motion::AnimProp::ScrollX {
            over_x
        } else {
            over_y
        };
        if overscroll.abs() > f32::EPSILON {
            // The overscroll spring owns this axis until it returns to the edge.
            return;
        }
        let Ok((x, y)) = self.scroll_offset(owner) else {
            return;
        };
        let x0 = if prop == crate::motion::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.animate(
            owner,
            prop,
            Box::new(crate::motion::Friction {
                x0,
                v0: sim_v,
                tau_ms: self.scroll_tau(owner),
            }),
        );
    }

    /// The scroll that should coast. Walks ancestors when `id` is already at the edge.
    fn fling_owner(&self, id: NodeId, prop: crate::motion::AnimProp, sim_v: f32) -> NodeId {
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
    fn can_take_velocity(&self, id: NodeId, prop: crate::motion::AnimProp, sim_v: f32) -> bool {
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
        let (offset, limit) = if prop == crate::motion::AnimProp::ScrollX {
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
        prop: crate::motion::AnimProp,
        velocity: f32,
        snap: bool,
    ) {
        if !snap && velocity.abs() >= 0.02 && self.axis_blocked(id, prop, velocity) {
            if self.handoff_fling(id, prop, velocity) {
                self.note_scroll_phase(id, crate::motion::ScrollPhase::Idle, 0.0, 0.0);
                return;
            }
            if self.bounce_fling(id, prop, velocity) {
                self.note_scroll_phase(id, crate::motion::ScrollPhase::Settling, 0.0, 0.0);
                return;
            }
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
                self.note_scroll_phase(id, crate::motion::ScrollPhase::Settling, 0.0, 0.0);
                return;
            }
        }
        self.sync_scroll_phase(id);
    }

    /// Leftover coast velocity becomes an overscroll spring at the wall.
    /// `sim_v` is the offset derivative, the same sign `Friction` used.
    fn bounce_fling(&mut self, id: NodeId, prop: crate::motion::AnimProp, sim_v: f32) -> bool {
        let physics = self.scroll_physics(id).unwrap_or_default();
        self.bounce_at_edge(id, prop, sim_v, physics)
    }

    pub(super) fn bounce_wheel(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        velocity: f32,
    ) {
        let physics = self.scroll_physics(id).unwrap_or_default().for_trackpad();
        self.bounce_at_edge(id, prop, velocity, physics);
    }

    fn bounce_at_edge(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        sim_v: f32,
        physics: crate::motion::ScrollPhysics,
    ) -> bool {
        if physics.overscroll == crate::motion::OverscrollMode::Clamp {
            return false;
        }
        let Ok(node) = self.node(id) else {
            return false;
        };
        let (ox, oy) = node.overscroll();
        let (max_x, max_y) = self.scroll_limits(id);
        let (over_prop, x0, dimension, limit) = if prop == crate::motion::AnimProp::ScrollX {
            (crate::motion::AnimProp::OverscrollX, ox, node.w, max_x)
        } else {
            (crate::motion::AnimProp::OverscrollY, oy, node.h, max_y)
        };
        if physics.accept == crate::motion::ScrollAccept::Overflow && limit <= 0.5 {
            return false;
        }
        if x0.abs() > f32::EPSILON {
            // Already past the wall. Pointer-up armed the return spring;
            // restarting it here would freeze the stretch on this sample.
            return false;
        }
        // From rest position, a critical spring peaks at v0 / (e * ω).
        // Use the undamped energy bound for softer damping.
        let omega = physics.overscroll_stiffness.sqrt();
        let max_peak = (dimension * 0.15).min(72.0);
        let gain = if physics.overscroll_damping >= 2.0 * omega {
            std::f32::consts::E
        } else {
            1.0
        };
        let speed_limit = max_peak * omega * gain / 1000.0;
        if !speed_limit.is_finite() || speed_limit <= 0.0 {
            return false;
        }
        let v0 = (sim_v * physics.band.slope(0.0, dimension)).clamp(-speed_limit, speed_limit);
        if !v0.is_finite() || v0.abs() < 0.02 {
            return false;
        }
        if self
            .motions
            .iter()
            .any(|motion| motion.node == id && motion.prop == over_prop)
        {
            return true;
        }
        self.arm_overscroll_axis(id, over_prop, x0, v0, physics);
        true
    }

    fn handoff_fling(&mut self, id: NodeId, prop: crate::motion::AnimProp, sim_v: f32) -> bool {
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
        let x0 = if prop == crate::motion::AnimProp::ScrollX {
            x
        } else {
            y
        };
        self.animate(
            owner,
            prop,
            Box::new(crate::motion::Friction {
                x0,
                v0: sim_v,
                tau_ms: self.scroll_tau(owner),
            }),
        );
        let (vx, vy) = if prop == crate::motion::AnimProp::ScrollX {
            (sim_v, 0.0)
        } else {
            (0.0, sim_v)
        };
        self.note_scroll_phase(owner, crate::motion::ScrollPhase::Coasting, vx, vy);
        true
    }

    fn axis_blocked(&self, id: NodeId, prop: crate::motion::AnimProp, sim_v: f32) -> bool {
        !self.can_take_velocity(id, prop, sim_v)
    }

    fn axis_offset(&self, id: NodeId, prop: crate::motion::AnimProp) -> f32 {
        let Ok((x, y)) = self.scroll_offset(id) else {
            return 0.0;
        };
        if prop == crate::motion::AnimProp::ScrollX {
            x
        } else {
            y
        }
    }

    fn snap_target(&self, id: NodeId, prop: crate::motion::AnimProp) -> Option<f32> {
        if !self.axis_of(id).allows(prop) {
            return None;
        }
        let physics = self.scroll_physics(id)?;
        let grid = match prop {
            crate::motion::AnimProp::ScrollX => physics.snap_x,
            crate::motion::AnimProp::ScrollY => physics.snap_y,
            _ => None,
        }?;
        if !(grid.interval.is_finite() && grid.interval > 0.5) {
            return None;
        }
        let (offset_x, offset_y) = self.scroll_offset(id).ok()?;
        let node = self.node(id).ok()?;
        let (offset, view) = if prop == crate::motion::AnimProp::ScrollX {
            (offset_x, node.w)
        } else {
            (offset_y, node.h)
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let limit = if prop == crate::motion::AnimProp::ScrollX {
            max_x.max(0.0)
        } else {
            max_y.max(0.0)
        };
        let origin = match grid.align {
            crate::motion::SnapAlign::Start => offset,
            crate::motion::SnapAlign::Center => offset + view * 0.5,
            crate::motion::SnapAlign::End => offset + view,
        };
        let index = (origin / grid.interval).round();
        let snapped = index * grid.interval;
        let next = match grid.align {
            crate::motion::SnapAlign::Start => snapped,
            crate::motion::SnapAlign::Center => snapped - view * 0.5,
            crate::motion::SnapAlign::End => snapped - view,
        };
        Some(next.clamp(0.0, limit))
    }

    pub(super) fn sync_scroll_phase(&mut self, id: NodeId) {
        let coasting = self.motions.iter().any(|motion| {
            motion.node == id
                && matches!(
                    motion.prop,
                    crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY
                )
                && !motion.snap
        });
        let settling = self.motions.iter().any(|motion| {
            motion.node == id
                && (motion.snap
                    || matches!(
                        motion.prop,
                        crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY
                    ))
        });
        let phase = if coasting {
            crate::motion::ScrollPhase::Coasting
        } else if settling {
            crate::motion::ScrollPhase::Settling
        } else {
            crate::motion::ScrollPhase::Idle
        };
        self.note_scroll_phase(id, phase, 0.0, 0.0);
    }

    pub(super) fn note_scroll_phase(
        &mut self,
        id: NodeId,
        phase: crate::motion::ScrollPhase,
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
            let offset_y = self.scroll_offset(id).map(|pair| pair.1).unwrap_or(0.0);
            tracing::info!(
                target: "impasto::gesture",
                scroll = id.to_raw(),
                phase = phase.name(),
                vx,
                vy,
                offset_y,
                overscroll_y = oy,
                "scroll phase"
            );
            self.scroll_notices.push(Gesture::ScrollPhaseChanged {
                id,
                phase,
                vx,
                vy,
                ox,
                oy,
            });
            if phase == crate::motion::ScrollPhase::Idle {
                self.commit_overscroll_pull(id);
            }
        }
    }
}

fn is_scroll_motion(prop: crate::motion::AnimProp) -> bool {
    matches!(
        prop,
        crate::motion::AnimProp::ScrollX
            | crate::motion::AnimProp::ScrollY
            | crate::motion::AnimProp::OverscrollX
            | crate::motion::AnimProp::OverscrollY
    )
}

impl Tree {
    /// Snap a scroll's offsets to whole device pixels after its motion ends.
    /// A no-op at the default pixel scale of one. Overscroll stays; a fling
    /// that just hit the wall still has leftover velocity to bounce.
    pub(super) fn settle_scroll_pixels(&mut self, id: NodeId) {
        if (self.pixel_scale - 1.0).abs() <= f32::EPSILON {
            return;
        }
        let Ok((x, y)) = self.scroll_offset(id) else {
            return;
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let axis = self.axis_of(id);
        let nx = if axis.allows_x() {
            self.settle_pixels(x).clamp(0.0, max_x.max(0.0))
        } else {
            x
        };
        let ny = if axis.allows_y() {
            self.settle_pixels(y).clamp(0.0, max_y.max(0.0))
        } else {
            y
        };
        if (nx - x).abs() <= f32::EPSILON && (ny - y).abs() <= f32::EPSILON {
            return;
        }
        let Some(geom) = self.node_mut(id).ok().and_then(|node| node.scroll_mut()) else {
            return;
        };
        geom.offset_x = nx;
        geom.offset_y = ny;
        self.canvas_geometry_changed();
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }
}
