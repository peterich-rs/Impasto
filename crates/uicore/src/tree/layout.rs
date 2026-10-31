//! Taffy styles, measure, and the viewport.

use std::cell::Cell;
use std::sync::Arc;

use taffy::prelude::*;

use super::node::{Kind, NodeFlags, Slot, SubtreeSpace};
use super::{Alignment, AxisSize, Edges, GridLine, Inset, Justify, NodeId, Rect, TrackSize, Tree};
use crate::{Error, SINGLE_LINE_MAX, Shaper, TextSize};

impl Tree {
    /// Content box for the next `layout`, in scene points.
    ///
    /// `(x, y)` is where the root is placed. `width` and `height` are the space
    /// Taffy may use. Unset size keeps `MAX_CONTENT`. Does not drop the previous
    /// geometry. Hit testing uses that geometry until `layout` runs again.
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32) -> Result<(), Error> {
        finite_number(x)?;
        finite_number(y)?;
        finite_number(width)?;
        finite_number(height)?;
        self.origin = (x, y);
        self.viewport = Some((width.max(0.0), height.max(0.0)));
        self.note_layout(LayoutCause::Viewport);
        if let Some(root) = self.root {
            self.mark_layout_from(root);
            self.dirty_every_picture();
        } else {
            self.mark_layout();
        }
        Ok(())
    }

    /// True when the next `layout` must run Taffy.
    pub fn is_dirty(&self) -> bool {
        self.needs_layout
    }

    pub fn needs_layout(&self) -> bool {
        self.needs_layout
    }

    pub fn needs_paint(&self) -> bool {
        self.needs_paint
    }

    pub fn layout_passes(&self) -> u64 {
        self.layout_passes
    }

    pub(super) fn note_layout(&mut self, cause: LayoutCause) {
        if self.layout_cause == LayoutCause::None {
            self.layout_cause = cause;
        }
    }

    /// Layout counters since the previous take. The next frame starts at zero.
    pub fn take_layout_trace(&mut self) -> LayoutTrace {
        let trace = LayoutTrace {
            laywork_ms: self.layout_work_ms,
            passes: self.layout_entries,
            text_reused: self.text_reused.take(),
            text_shaped: self.text_shaped.take(),
            cause: self.layout_cause,
        };
        self.layout_work_ms = 0.0;
        self.layout_entries = 0;
        self.layout_cause = LayoutCause::None;
        trace
    }

    pub(super) fn mark_layout(&mut self) {
        self.needs_layout = true;
        self.needs_paint = true;
    }

    pub(super) fn mark_paint(&mut self) {
        self.needs_paint = true;
    }

    /// Diries the picture that owns `id`. A layer also dirties its parent picture.
    pub(super) fn mark_repaint(&mut self, id: NodeId) {
        self.dirty_picture(id);
        if self.renders_layer_node(id)
            && let Some(parent) = self.get(id).and_then(|node| node.parent)
        {
            self.dirty_picture(parent);
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    /// A layer property changed and the run commands did not.
    pub(super) fn paint_layer_value(&mut self) {
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    /// The open-layer bracket flipped. Dirty every ancestor so outer runs rebuild.
    ///
    /// Recomputes `subtree_layers` first. A clip's layer bit depends on descendants,
    /// and the map from the previous frame would stop the walk too early.
    pub(super) fn repaint_ancestors(&mut self, id: NodeId) {
        if let Some(root) = self.root {
            self.subtree_layers.clear();
            let _ = self.sweep_subtree_layers(root);
        }
        let mut current = Some(id);
        while let Some(node_id) = current {
            if let Ok(node) = self.node_mut(node_id) {
                node.flags.insert(NodeFlags::REPAINT_DIRTY);
            }
            current = self.get(node_id).and_then(|node| node.parent);
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    pub(super) fn dirty_picture(&mut self, mut id: NodeId) {
        loop {
            let parent = match self.node(id) {
                Ok(node) => node.parent,
                Err(_) => return,
            };
            let stop = parent.is_none() || self.renders_layer_node(id);
            if stop {
                if let Ok(node) = self.node_mut(id) {
                    node.flags.insert(NodeFlags::REPAINT_DIRTY);
                }
                return;
            }
            let Some(parent) = parent else {
                return;
            };
            id = parent;
        }
    }

    pub(super) fn renders_layer_node(&self, id: NodeId) -> bool {
        let subtree = self.subtree_layers.get(&id).copied().unwrap_or(false);
        self.get(id)
            .is_some_and(|node| node.renders_layers(subtree))
    }

    pub(super) fn is_repaint_dirty(&self, id: NodeId) -> bool {
        self.get(id)
            .is_some_and(|node| node.flags.contains(NodeFlags::REPAINT_DIRTY))
    }

    pub(super) fn dirty_every_picture(&mut self) {
        let Some(root) = self.root else {
            return;
        };
        self.subtree_layers.clear();
        let _ = self.sweep_subtree_layers(root);
        let ids: Vec<NodeId> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                slot.node.as_ref()?;
                Some(NodeId {
                    index: crate::cast::u32_from_usize(index),
                    generation: slot.generation,
                })
            })
            .collect();
        let owners: Vec<NodeId> = ids
            .into_iter()
            .filter(|id| {
                self.get(*id).is_some_and(|node| node.parent.is_none())
                    || self.renders_layer_node(*id)
            })
            .collect();
        for id in owners {
            if let Ok(node) = self.node_mut(id) {
                node.flags.insert(NodeFlags::REPAINT_DIRTY);
            }
        }
    }

    pub fn clear_paint(&mut self) {
        self.needs_paint = false;
    }

    /// Foreground for text, fill for a decorated box.
    pub fn set_color(&mut self, id: NodeId, color: [u8; 4]) -> Result<(), Error> {
        let kind = self.node(id)?.kind;
        match kind {
            Kind::Text => {
                {
                    let node = self.node_mut(id)?;
                    let Some(text) = node.text_mut() else {
                        return Err(Error::NotText);
                    };
                    if text.color == color {
                        return Ok(());
                    }
                    text.color = color;
                }
                self.touch_measure(id)
            }
            Kind::Decorated => {
                self.node_mut(id)?
                    .write_paint(|paint| paint.fill = Some(color));
                self.mark_repaint(id);
                Ok(())
            }
            _ => Err(Error::WrongKind),
        }
    }

    pub fn set_gap(&mut self, id: NodeId, gap: f32) -> Result<(), Error> {
        finite_number(gap)?;
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.gap = Size {
            width: length(gap),
            height: length(gap),
        };
        self.write_style(id, style)
    }

    /// Sets flex grow on this node. The value must be zero or positive.
    ///
    /// Marks the tree dirty and keeps the previous geometry until the next `layout`.
    /// Free space exists when the flex parent has a definite main-axis size.
    pub fn set_grow(&mut self, id: NodeId, grow: f32) -> Result<(), Error> {
        // NaN is not zero or positive. `+∞` stays legal.
        if grow < 0.0 || grow.is_nan() {
            return Err(Error::NegativeGrow);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_grow = grow;
        self.write_style(id, style)
    }

    pub fn set_padding(&mut self, id: NodeId, edges: f32) -> Result<(), Error> {
        self.set_padding_edges(id, Edges::all(edges))
    }

    /// Padding on a [`Kind::Padding`] node. A non-finite edge leaves the style unchanged.
    pub fn set_padding_edges(&mut self, id: NodeId, edges: Edges) -> Result<(), Error> {
        let padding = edge_rect(edges)?;
        if self.node(id)?.kind != Kind::Padding {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.padding = padding;
        self.write_style(id, style)
    }

    /// Margin on any node, in scene points.
    pub fn set_margin(&mut self, id: NodeId, edges: Edges) -> Result<(), Error> {
        let margin = margin_rect(edges)?;
        let mut style = self.node(id)?.style.clone();
        style.margin = margin;
        self.write_style(id, style)
    }

    pub fn set_min_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        {
            let style = &self.node(id)?.style;
            if style.min_size.width == length(width) && style.min_size.height == length(height) {
                return Ok(());
            }
        }
        let mut style = self.node(id)?.style.clone();
        style.min_size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)
    }

    /// Min size where either axis may be a percentage of the parent.
    /// `Fraction(0.5)` is `min-width: 50%`; `Auto` leaves that axis alone.
    pub fn set_min_axis(
        &mut self,
        id: NodeId,
        width: AxisSize,
        height: AxisSize,
    ) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        if width != AxisSize::Auto {
            style.min_size.width = axis_dimension(width)?;
        }
        if height != AxisSize::Auto {
            style.min_size.height = axis_dimension(height)?;
        }
        self.write_style(id, style)
    }

    /// Max size where either axis may be a percentage of the parent.
    /// `Fraction(0.8)` is `max-width: 80%`; `Auto` leaves that axis alone.
    pub fn set_max_axis(
        &mut self,
        id: NodeId,
        width: AxisSize,
        height: AxisSize,
    ) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        if width != AxisSize::Auto {
            style.max_size.width = axis_dimension(width)?;
        }
        if height != AxisSize::Auto {
            style.max_size.height = axis_dimension(height)?;
        }
        self.write_style(id, style)
    }

    pub fn set_max_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.max_size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)
    }

    pub fn set_shrink(&mut self, id: NodeId, shrink: f32) -> Result<(), Error> {
        finite_number(shrink)?;
        if shrink < 0.0 {
            return Err(Error::BadNumber);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_shrink = shrink;
        self.write_style(id, style)
    }

    pub fn set_basis(&mut self, id: NodeId, basis: f32) -> Result<(), Error> {
        let basis = finite_positive(basis)?;
        let mut style = self.node(id)?.style.clone();
        style.flex_basis = length(basis);
        self.write_style(id, style)
    }

    pub fn set_align_self(&mut self, id: NodeId, alignment: Alignment) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.align_self = Some(align_self(alignment));
        self.write_style(id, style)
    }

    /// Corner radius of a decorated box or a clip, in scene points.
    pub fn set_radius(&mut self, id: NodeId, radius: f32) -> Result<(), Error> {
        let radius = finite_positive(radius)?;
        let node = self.node_mut(id)?;
        if !matches!(node.kind, Kind::Decorated | Kind::Clip) {
            return Err(Error::WrongKind);
        }
        node.write_paint(|paint| paint.corner_radius = radius);
        let _ = node;
        self.mark_repaint(id);
        Ok(())
    }

    /// Layer alpha, 0–255. Geometry stays until the next layout.
    pub fn set_alpha(&mut self, id: NodeId, alpha: u8) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Opacity {
            return Err(Error::WrongKind);
        }
        let previous = alpha_bucket(self.node(id)?.alpha());
        self.node_mut(id)?
            .write_paint(|paint| paint.alpha = Some(alpha));
        let next = alpha_bucket(self.node(id)?.alpha());
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    pub fn set_align(&mut self, id: NodeId, alignment: Alignment) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Align {
            return Err(Error::WrongKind);
        }
        let (cross, main) = align_axes(alignment);
        let mut style = self.node(id)?.style.clone();
        style.align_items = Some(cross);
        style.justify_content = Some(main);
        self.write_style(id, style)
    }

    /// `true` writes `RowReverse`. Only a row accepts this.
    pub fn set_reverse(&mut self, id: NodeId, reverse: bool) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Row {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_direction = if reverse {
            FlexDirection::RowReverse
        } else {
            FlexDirection::Row
        };
        self.write_style(id, style)
    }

    /// Content width and height from the last [`Tree::set_viewport`].
    ///
    /// `(0, 0)` before the session receives metrics.
    pub fn viewport_size(&self) -> (f32, f32) {
        self.viewport.unwrap_or((0.0, 0.0))
    }

    /// Cross-axis alignment of a row or column.
    ///
    /// [`RowCrossAlign::Baseline`] on a column returns `Ok` and leaves the style
    /// alone. [`RowCrossAlign::Stretch`] writes `AlignItems::Stretch`.
    pub fn set_row_cross(&mut self, id: NodeId, align: super::RowCrossAlign) -> Result<(), Error> {
        let kind = self.node(id)?.kind;
        if !matches!(kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        if kind == Kind::Column && align == super::RowCrossAlign::Baseline {
            return Ok(());
        }
        let cross = match align {
            super::RowCrossAlign::Start | super::RowCrossAlign::Baseline => AlignItems::Start,
            super::RowCrossAlign::Center => AlignItems::Center,
            super::RowCrossAlign::End => AlignItems::End,
            super::RowCrossAlign::Stretch => AlignItems::Stretch,
        };
        self.node_mut(id)?.flags.set(
            NodeFlags::BASELINE_CROSS,
            align == super::RowCrossAlign::Baseline,
        );
        let mut style = self.node(id)?.style.clone();
        style.align_items = Some(cross);
        self.write_style(id, style)
    }

    /// Main-axis alignment of a row or column.
    pub fn set_justify(&mut self, id: NodeId, justify: Justify) -> Result<(), Error> {
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.justify_content = Some(justify_content(justify));
        self.write_style(id, style)
    }

    /// `true` lets a row or column wrap onto another line.
    pub fn set_wrap(&mut self, id: NodeId, wrap: bool) -> Result<(), Error> {
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_wrap = if wrap {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        };
        self.write_style(id, style)
    }

    /// Width and height become 100% of the parent, or of the viewport when this is the root.
    ///
    /// The minimum size is zero so a flex parent can shrink this node.
    /// Inside a row or column, `flex-basis` is zero and `flex-grow` is one,
    /// so the node takes the space left after its siblings. A filled stack,
    /// clip, opacity, or transform uses one fraction track, so its child uses
    /// that box instead of growing with the content. A scroll in that child
    /// then keeps the box as its viewport.
    pub fn set_fill(&mut self, id: NodeId) -> Result<(), Error> {
        let kind = self.node(id)?.kind;
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        };
        style.min_size = Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        };
        let parent_is_flex = self
            .node(id)?
            .parent
            .and_then(|parent| self.node(parent).ok())
            .is_some_and(|parent| matches!(parent.kind, Kind::Column | Kind::Row));
        if parent_is_flex {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = length(0.0_f32);
        }
        if matches!(
            kind,
            Kind::Stack | Kind::Opacity | Kind::Transform | Kind::Clip
        ) {
            style.grid_template_columns = vec![fr(1.0_f32)];
            style.grid_template_rows = vec![fr(1.0_f32)];
        }
        self.write_style(id, style)
    }

    pub fn set_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Sized {
            return Err(Error::WrongKind);
        }
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let width = length(width);
        let height = length(height);
        {
            let style = &self.node(id)?.style;
            if style.size.width == width && style.size.height == height {
                return Ok(());
            }
        }
        let mut style = self.node(id)?.style.clone();
        style.size = Size { width, height };
        self.write_style(id, style)
    }

    /// Width and height for any styled node. Does not change [`Tree::set_size`].
    pub fn set_axis_size(
        &mut self,
        id: NodeId,
        width: AxisSize,
        height: AxisSize,
    ) -> Result<(), Error> {
        let width = axis_dimension(width)?;
        let height = axis_dimension(height)?;
        {
            let style = &self.node(id)?.style;
            if style.size.width == width && style.size.height == height {
                return Ok(());
            }
        }
        let mut style = self.node(id)?.style.clone();
        style.size = Size { width, height };
        self.write_style(id, style)
    }

    /// Width divided by height. `None` clears it. Zero is not a clear.
    pub fn set_aspect_ratio(&mut self, id: NodeId, ratio: Option<f32>) -> Result<(), Error> {
        if let Some(ratio) = ratio
            && !(ratio.is_finite() && ratio > 0.0)
        {
            return Err(Error::BadNumber);
        }
        let mut style = self.node(id)?.style.clone();
        style.aspect_ratio = ratio;
        self.write_style(id, style)
    }

    /// `true` writes `Position::Absolute`. `false` restores `Relative`.
    pub fn set_position(&mut self, id: NodeId, absolute: bool) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.position = if absolute {
            Position::Absolute
        } else {
            Position::Relative
        };
        self.write_style(id, style)
    }

    /// Four edges, in scene points. `None` is auto. A number may be negative.
    pub fn set_inset(&mut self, id: NodeId, inset: Inset) -> Result<(), Error> {
        let top = inset_edge(inset.top)?;
        let right = inset_edge(inset.right)?;
        let bottom = inset_edge(inset.bottom)?;
        let left = inset_edge(inset.left)?;
        let mut style = self.node(id)?.style.clone();
        style.inset = taffy::Rect {
            top,
            right,
            bottom,
            left,
        };
        self.write_style(id, style)
    }

    /// Places a child of a [`Kind::Grid`]. Other parents, including stack-like grids, return [`Error::WrongKind`].
    pub fn set_grid_placement(
        &mut self,
        id: NodeId,
        column_start: GridLine,
        column_end: GridLine,
        row_start: GridLine,
        row_end: GridLine,
    ) -> Result<(), Error> {
        let Some(parent) = self.node(id)?.parent else {
            return Err(Error::WrongKind);
        };
        if self.node(parent)?.kind != Kind::Grid {
            return Err(Error::WrongKind);
        }
        let start_column = grid_placement(column_start)?;
        let end_column = grid_placement(column_end)?;
        let start_row = grid_placement(row_start)?;
        let end_row = grid_placement(row_end)?;
        let mut style = self.node(id)?.style.clone();
        style.grid_column = Line {
            start: start_column,
            end: end_column,
        };
        style.grid_row = Line {
            start: start_row,
            end: end_row,
        };
        self.write_style(id, style)
    }

    pub fn set_image(
        &mut self,
        id: NodeId,
        source: &str,
        width: f32,
        height: f32,
    ) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let previous = {
            let node = self.node_mut(id)?;
            let Some(image) = node.image_mut() else {
                return Err(Error::WrongKind);
            };
            let previous = Arc::clone(&image.source);
            image.source = Arc::from(source);
            image.width = width;
            image.height = height;
            previous
        };
        if previous.as_ref() != source {
            self.note_image(&previous, -1);
            self.note_image(&Arc::from(source), 1);
        }
        self.touch_measure(id)
    }

    /// Fit mode and whether a missing decode fails the frame. The source string stays `asset:name`.
    pub fn set_image_mode(
        &mut self,
        id: NodeId,
        mode: protocol::ContentMode,
        required: bool,
    ) -> Result<(), Error> {
        let node = self.node_mut(id)?;
        let Some(image) = node.image_mut() else {
            return Err(Error::WrongKind);
        };
        image.mode = mode;
        image.required = required;
        let _ = node;
        self.mark_repaint(id);
        Ok(())
    }

    /// Writes each motion's end value, then drops it.
    ///
    /// Springs land on `target`. Friction lands on `x0 + v0 * tau`, clamped to
    /// the scroll limits. Overscroll lands on zero.
    pub fn finish_motions(&mut self) {
        let motions = std::mem::take(&mut self.motions);
        for motion in motions {
            self.motion_ends.push((motion.node, motion.prop));
            let value = match motion.prop {
                crate::AnimProp::OverscrollX | crate::AnimProp::OverscrollY => 0.0,
                crate::AnimProp::ScrollX | crate::AnimProp::ScrollY => {
                    let (max_x, max_y) = self.scroll_limits(motion.node);
                    let limit = if motion.prop == crate::AnimProp::ScrollX {
                        max_x
                    } else {
                        max_y
                    };
                    motion.simulation.terminal().clamp(0.0, limit.max(0.0))
                }
                _ => motion.simulation.terminal(),
            };
            let _ = self.apply_motion(motion.node, motion.prop, value);
        }
    }

    /// Samples every active motion at `target_ms`. Returns whether any remain.
    pub fn tick_motions(&mut self, target_ms: f64) -> bool {
        if !target_ms.is_finite() {
            return !self.motions.is_empty();
        }
        self.clock_ms = target_ms;
        let mut index = 0;
        while index < self.motions.len() {
            let started = self.motions[index].started_ms;
            let t = (target_ms - started).max(0.0);
            let value = self.motions[index].simulation.x(t);
            let finished = self.motions[index].simulation.done(t);
            let velocity = self.motions[index].simulation.dx(t);
            let node = self.motions[index].node;
            let prop = self.motions[index].prop;
            let snap = self.motions[index].snap;
            let stop = self.apply_motion(node, prop, value) || finished;
            if stop {
                self.motion_ends.push((node, prop));
                self.motions.remove(index);
                if matches!(prop, crate::AnimProp::ScrollX | crate::AnimProp::ScrollY) {
                    self.settle_scroll_pixels(node);
                    self.after_scroll_motion(node, prop, velocity, snap);
                } else if matches!(
                    prop,
                    crate::AnimProp::OverscrollX | crate::AnimProp::OverscrollY
                ) {
                    // The sample that crosses the settle threshold is not zero.
                    // Land on the target, then let the phase notice carry that zero.
                    let _ = self.apply_overscroll(node, prop, 0.0);
                    self.sync_scroll_phase(node);
                }
            } else {
                index += 1;
            }
        }
        !self.motions.is_empty()
    }

    /// Motions removed by [`Tree::tick_motions`] or [`Tree::finish_motions`] since the last take.
    pub fn take_motion_ends(&mut self) -> Vec<(NodeId, crate::AnimProp)> {
        std::mem::take(&mut self.motion_ends)
    }

    /// Replaces any motion of `prop` on `id`. The sample clock is the last tick.
    pub fn animate(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        simulation: Box<dyn crate::Simulation>,
    ) {
        self.motions
            .retain(|motion| !(motion.node == id && motion.prop == prop));
        self.motions.push(super::ActiveMotion {
            started_ms: self.clock_ms,
            node: id,
            prop,
            simulation,
            snap: false,
        });
    }

    pub(super) fn animate_snap(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        simulation: Box<dyn crate::Simulation>,
    ) {
        self.animate(id, prop, simulation);
        if let Some(motion) = self
            .motions
            .iter_mut()
            .rev()
            .find(|motion| motion.node == id && motion.prop == prop)
        {
            motion.snap = true;
        }
    }

    pub(super) fn apply_motion(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
        self.applying_motion = true;
        let stop = match prop {
            crate::AnimProp::ScrollX | crate::AnimProp::ScrollY => {
                self.apply_scroll(id, prop, value)
            }
            crate::AnimProp::Opacity => {
                let rounded = value.round().clamp(0.0, 255.0);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "rounded and clamped into the closed range 0..=255"
                )]
                let alpha = rounded as u8;
                self.set_alpha(id, alpha).is_err()
            }
            crate::AnimProp::TranslateX
            | crate::AnimProp::TranslateY
            | crate::AnimProp::Rotate
            | crate::AnimProp::Scale => self.apply_transform_prop(id, prop, value),
            crate::AnimProp::OverscrollX | crate::AnimProp::OverscrollY => {
                self.apply_overscroll(id, prop, value)
            }
        };
        self.applying_motion = false;
        stop
    }

    pub(super) fn apply_transform_prop(
        &mut self,
        id: NodeId,
        prop: crate::AnimProp,
        value: f32,
    ) -> bool {
        let Ok(node) = self.node(id) else {
            return true;
        };
        let current = node.transform();
        let mut translate_x = current.translate_x;
        let mut translate_y = current.translate_y;
        let mut rotate = current.rotate_rad;
        let mut scale = current.scale_x;
        match prop {
            crate::AnimProp::TranslateX => translate_x = value,
            crate::AnimProp::TranslateY => translate_y = value,
            crate::AnimProp::Rotate => rotate = value,
            crate::AnimProp::Scale => scale = value,
            _ => return true,
        }
        self.set_transform(id, translate_x, translate_y, rotate, scale)
            .is_err()
    }

    /// Paint transform. Scale is uniform. Rotation is clockwise radians.
    ///
    /// Does not mark Taffy dirty. Hit testing inverts this transform.
    pub fn set_transform(
        &mut self,
        id: NodeId,
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale: f32,
    ) -> Result<(), Error> {
        finite_number(translate_x)?;
        finite_number(translate_y)?;
        finite_number(rotate_rad)?;
        finite_number(scale)?;
        if self.node(id)?.kind == Kind::Transform {
            let geom = self.node(id)?.transform();
            if geom.translate_x == translate_x
                && geom.translate_y == translate_y
                && geom.rotate_rad == rotate_rad
                && geom.scale_x == scale
                && geom.scale_y == scale
            {
                return Ok(());
            }
        }
        let was_identity = self.node(id)?.transform().is_identity();
        {
            let node = self.node_mut(id)?;
            let Some(geom) = node.transform_mut() else {
                return Err(Error::WrongKind);
            };
            geom.translate_x = translate_x;
            geom.translate_y = translate_y;
            geom.rotate_rad = rotate_rad;
            geom.scale_x = scale;
            geom.scale_y = scale;
        }
        let is_identity = self.node(id)?.transform().is_identity();
        if was_identity == is_identity {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    pub fn layout(&mut self) -> Result<(), Error> {
        if !self.needs_layout && self.laid_out {
            return Ok(());
        }
        let root = self.root.ok_or(Error::NoRoot)?;
        let root_taffy = self.node(root)?.taffy.ok_or(Error::NoRoot)?;
        let available = match self.viewport {
            Some((width, height)) => Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            None => Size::MAX_CONTENT,
        };
        let started = std::time::Instant::now();
        let mut measured = Vec::new();
        let env = MeasureEnv {
            slots: &self.slots,
            shaper: self.shaper.as_ref(),
            text_scale: self.text_scale,
            text_reused: &self.text_reused,
            text_shaped: &self.text_shaped,
        };
        let taffy_started = std::time::Instant::now();
        self.taffy.compute_layout_with_measure(
            root_taffy,
            available,
            |known, available, _id, context, _style| {
                measure_leaf(&env, known, available, context, &mut measured)
            },
        )?;
        self.taffy_ms = taffy_started.elapsed().as_secs_f64() * 1000.0;
        for id in measured {
            self.note_text(id);
        }
        let boundaries = std::mem::take(&mut self.dirty_boundaries);
        if boundaries.is_empty() {
            self.place_boundary(root)?;
        } else {
            for id in boundaries {
                let dirty = self
                    .node(id)
                    .ok()
                    .is_some_and(|node| node.flags.contains(NodeFlags::BOUNDARY_DIRTY));
                if dirty {
                    self.place_boundary(id)?;
                }
            }
        }
        self.layout_passes = self.layout_passes.saturating_add(1);
        self.layout_entries = self.layout_entries.saturating_add(1);
        self.needs_layout = false;
        self.needs_paint = true;
        self.laid_out = true;
        self.sync_paragraphs();
        self.layout_work_ms += started.elapsed().as_secs_f64() * 1000.0;
        Ok(())
    }

    /// The pointer region wins a drag on the claimed axis before scrolling does.
    pub fn set_drag_claim(&mut self, id: NodeId, claim: super::DragClaim) -> Result<(), Error> {
        self.node_mut(id)?.drag_claim = claim;
        Ok(())
    }

    /// Mark `id` as a relayout boundary so its subtree lays out independently.
    /// Popovers and dropdowns declare this before [`Tree::measure_panel`] so
    /// the measurement does not dirty the rest of the tree.
    pub fn set_panel(&mut self, id: NodeId) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        // A boundary needs definite inputs on the panel itself; pin its own
        // size so the subtree resolves against a stable box.
        style.overflow = taffy::Point {
            x: taffy::Overflow::Visible,
            y: taffy::Overflow::Visible,
        };
        self.write_style(id, style)?;
        if let Ok(node) = self.node_mut(id) {
            node.flags.insert(NodeFlags::RELAYOUT_BOUNDARY);
        }
        Ok(())
    }

    /// Lay out `id`'s subtree under `space` and return the content size.
    ///
    /// The caller then pins the panel with [`Tree::set_axis_size`] so the
    /// recorded frame uses the measured box. `MaxContent` measures naturally,
    /// `CappedMax` re-lays out with the cap when the natural size exceeds it,
    /// and `Definite` lays out with the given width.
    pub fn measure_panel(&mut self, id: NodeId, space: SubtreeSpace) -> Result<(f32, f32), Error> {
        let taffy_id = self.node(id)?.taffy.ok_or(Error::NoRoot)?;
        let (first_width, recheck_cap) = match space {
            SubtreeSpace::MaxContent | SubtreeSpace::CappedMax(_) => {
                (AvailableSpace::MaxContent, None)
            }
            SubtreeSpace::Definite { width, .. } => (AvailableSpace::Definite(width), Some(width)),
        };
        let first_height = match space {
            SubtreeSpace::Definite {
                height: Some(h), ..
            } => AvailableSpace::Definite(h),
            _ => AvailableSpace::MaxContent,
        };
        let started = std::time::Instant::now();
        let mut measured = Vec::new();
        let env = MeasureEnv {
            slots: &self.slots,
            shaper: self.shaper.as_ref(),
            text_scale: self.text_scale,
            text_reused: &self.text_reused,
            text_shaped: &self.text_shaped,
        };
        let mut run = |width: AvailableSpace, height: AvailableSpace| -> Result<Size<f32>, Error> {
            self.taffy.compute_layout_with_measure(
                taffy_id,
                Size { width, height },
                |known, available, _id, context, _style| {
                    measure_leaf(&env, known, available, context, &mut measured)
                },
            )?;
            Ok(self.taffy.layout(taffy_id)?.size)
        };
        let natural = run(first_width, first_height)?;
        let mut size = natural;
        if let SubtreeSpace::CappedMax(cap) = space {
            if natural.width > cap {
                size = run(AvailableSpace::Definite(cap), AvailableSpace::MaxContent)?;
            }
        } else if let Some(cap) = recheck_cap {
            let _ = cap;
        }
        for id in measured {
            self.note_text(id);
        }
        if let Ok(node) = self.node_mut(id) {
            node.flags.insert(NodeFlags::BOUNDARY_DIRTY);
        }
        self.place_boundary(id)?;
        self.layout_work_ms += started.elapsed().as_secs_f64() * 1000.0;
        Ok((size.width, size.height))
    }

    pub fn rect(&self, id: NodeId) -> Result<Rect, Error> {
        let (x, y) = self.layout_point(id)?;
        let node = self.node(id)?;
        Ok(Rect {
            x,
            y,
            w: node.w,
            h: node.h,
        })
    }

    /// Scene position of the border box. Scroll and transform stay out of this sum.
    pub(super) fn layout_point(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut current = Some(id);
        while let Some(cid) = current {
            let node = self.node(cid)?;
            x += node.x;
            y += node.y;
            current = node.parent;
        }
        Ok((x + self.origin.0, y + self.origin.1))
    }

    pub(super) fn write_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy;
        let kind = self.node(id)?.kind;
        let is_root = self.root == Some(id);
        {
            let node = self.node_mut(id)?;
            node.style = style.clone();
            let points = is_length(style.size.width) && is_length(style.size.height);
            node.flags.set(
                NodeFlags::RELAYOUT_BOUNDARY,
                kind == Kind::Scroll || is_root || points,
            );
        }
        if let Some(taffy_id) = taffy_id {
            self.taffy.set_style(taffy_id, style)?;
        }
        self.note_layout(LayoutCause::Style);
        self.mark_layout_from(id);
        Ok(())
    }

    pub(super) fn touch_measure(&mut self, id: NodeId) -> Result<(), Error> {
        self.note_layout(LayoutCause::Text);
        self.note_text(id);
        if let Some(taffy_id) = self.node(id)?.taffy {
            self.taffy.mark_dirty(taffy_id)?;
        }
        self.mark_layout_from(id);
        Ok(())
    }

    pub fn layout_timing(&self) -> (f64, f64) {
        (self.taffy_ms, self.sync_ms)
    }

    /// Marks `id` and the nearest relayout boundary. The boundary includes `id`
    /// when `id` itself is one.
    pub(super) fn mark_layout_from(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags.insert(NodeFlags::LAYOUT_DIRTY);
        }
        self.mark_boundary(id);
        self.mark_layout();
        self.mark_repaint(id);
    }

    pub(super) fn mark_boundary(&mut self, mut id: NodeId) {
        loop {
            let Ok(node) = self.node(id) else {
                return;
            };
            let parent = node.parent;
            let boundary = node.flags.contains(NodeFlags::RELAYOUT_BOUNDARY);
            let is_root = parent.is_none() && self.root == Some(id);
            if boundary || parent.is_none() {
                if let Ok(node) = self.node_mut(id) {
                    node.flags.insert(NodeFlags::BOUNDARY_DIRTY);
                    if is_root {
                        node.flags.insert(NodeFlags::RELAYOUT_BOUNDARY);
                    }
                }
                if !self.dirty_boundaries.contains(&id) {
                    self.dirty_boundaries.push(id);
                }
                return;
            }
            let Some(parent) = parent else {
                return;
            };
            id = parent;
        }
    }

    pub(super) fn place_boundary(&mut self, id: NodeId) -> Result<(), Error> {
        self.write_box_from_taffy(id)?;
        let mut child = self.node(id)?.first_child;
        while let Some(current) = child {
            let next = self.node(current)?.next;
            self.write_box_from_taffy(current)?;
            let flags = self.node(current)?.flags;
            let descend = flags.contains(NodeFlags::LAYOUT_DIRTY)
                || !flags.contains(NodeFlags::RELAYOUT_BOUNDARY);
            if descend {
                self.place_boundary(current)?;
            }
            child = next;
        }
        if self.node(id)?.flags.contains(NodeFlags::BASELINE_CROSS)
            && self.node(id)?.kind == Kind::Row
        {
            self.shift_baseline(id)?;
        }
        self.clear_layout_flags(id);
        Ok(())
    }

    pub(super) fn write_box_from_taffy(&mut self, id: NodeId) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy.ok_or(Error::UnknownNode)?;
        let (x, y, w, h) = {
            let layout = self.taffy.layout(taffy_id)?;
            (
                layout.location.x,
                layout.location.y,
                layout.size.width,
                layout.size.height,
            )
        };
        let node = self.node_mut(id)?;
        node.x = x;
        node.y = y;
        node.w = w;
        node.h = h;
        Ok(())
    }

    pub(super) fn shift_baseline(&mut self, row: NodeId) -> Result<(), Error> {
        let mut child = self.node(row)?.first_child;
        let mut samples = Vec::new();
        let mut max_baseline = 0.0f32;
        while let Some(current) = child {
            let next = self.node(current)?.next;
            let baseline = self.node_baseline(current);
            max_baseline = max_baseline.max(baseline);
            samples.push((current, baseline));
            child = next;
        }
        for (id, baseline) in samples {
            self.node_mut(id)?.y += max_baseline - baseline;
        }
        Ok(())
    }

    pub(super) fn node_baseline(&self, id: NodeId) -> f32 {
        let Ok(node) = self.node(id) else {
            return 0.0;
        };
        if node.kind == Kind::Text {
            if let Some(metrics) = node.text().and_then(|text| text.metrics.as_ref()) {
                return metrics.first_baseline();
            }
            if let Ok(key) = self.text_key(id) {
                return self.shaper.metrics(&key.metrics).first_baseline();
            }
        }
        node.h
    }

    pub(super) fn clear_layout_flags(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags
                .remove(NodeFlags::LAYOUT_DIRTY | NodeFlags::BOUNDARY_DIRTY);
        }
        self.dirty_boundaries.retain(|boundary| *boundary != id);
    }
}

pub(super) fn rubber_band(x: f32, y: f32, max_x: f32, max_y: f32) -> (f32, f32) {
    (x.clamp(0.0, max_x.max(0.0)), y.clamp(0.0, max_y.max(0.0)))
}

pub(super) fn nearest_offset(current: f32, start: f32, end: f32, view: f32) -> f32 {
    if end - start <= view {
        if start < current {
            start
        } else if end > current + view {
            end - view
        } else {
            current
        }
    } else if start < current {
        start
    } else {
        end - view
    }
}

fn is_length(size: Dimension) -> bool {
    size.tag() == CompactLength::LENGTH_TAG
}

pub(super) fn live_node(slots: &[Slot], id: NodeId) -> Result<&super::node::Node, Error> {
    let slot = slots.get(id.slot()).ok_or(Error::UnknownNode)?;
    if slot.generation != id.generation {
        return Err(Error::UnknownNode);
    }
    slot.node.as_ref().ok_or(Error::UnknownNode)
}

/// 0 hides the subtree, 1 is an opacity layer, 2 draws into the parent run.
fn alpha_bucket(alpha: Option<u8>) -> u8 {
    match alpha {
        Some(0) => 0,
        Some(255) | None => 2,
        Some(_) => 1,
    }
}

pub(super) fn finite_number(value: f32) -> Result<(), Error> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(Error::BadNumber)
    }
}

/// Why the next `layout` has to run, plus how the last stretch measured text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutCause {
    #[default]
    None,
    Viewport,
    Style,
    Text,
    Structure,
}

impl LayoutCause {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Viewport => "viewport",
            Self::Style => "style",
            Self::Text => "text",
            Self::Structure => "structure",
        }
    }
}

/// Counters from [`Tree::take_layout_trace`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutTrace {
    pub laywork_ms: f64,
    pub passes: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    pub cause: LayoutCause,
}

pub(super) fn finite_positive(value: f32) -> Result<f32, Error> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(Error::BadNumber)
    }
}

fn axis_dimension(size: AxisSize) -> Result<Dimension, Error> {
    match size {
        AxisSize::Auto => Ok(auto()),
        AxisSize::Points(value) => Ok(length(finite_positive(value)?)),
        AxisSize::Fraction(value) => {
            finite_positive(value)?;
            Ok(percent(value))
        }
    }
}

fn inset_edge(edge: Option<f32>) -> Result<LengthPercentageAuto, Error> {
    match edge {
        None => Ok(auto()),
        Some(value) => {
            finite_number(value)?;
            Ok(length(value))
        }
    }
}

fn grid_placement(slot: GridLine) -> Result<GridPlacement, Error> {
    match slot {
        GridLine::Auto => Ok(GridPlacement::Auto),
        GridLine::Line(index) => {
            if index == 0 {
                return Err(Error::BadNumber);
            }
            Ok(line(index))
        }
        GridLine::Span(count) => {
            if count == 0 {
                return Err(Error::BadNumber);
            }
            Ok(span(count))
        }
    }
}

pub(super) fn align_axes(alignment: Alignment) -> (AlignItems, JustifyContent) {
    match alignment {
        Alignment::Start => (AlignItems::Start, JustifyContent::Start),
        Alignment::Center => (AlignItems::Center, JustifyContent::Center),
        Alignment::End => (AlignItems::End, JustifyContent::End),
    }
}

fn edges_rect<T: FromLength>(edges: Edges) -> Result<taffy::Rect<T>, Error> {
    finite_number(edges.top)?;
    finite_number(edges.right)?;
    finite_number(edges.bottom)?;
    finite_number(edges.left)?;
    Ok(taffy::Rect {
        left: length(edges.left),
        right: length(edges.right),
        top: length(edges.top),
        bottom: length(edges.bottom),
    })
}

fn edge_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentage>, Error> {
    edges_rect(edges)
}

fn margin_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentageAuto>, Error> {
    edges_rect(edges)
}

fn align_self(alignment: Alignment) -> AlignSelf {
    match alignment {
        Alignment::Start => AlignSelf::Start,
        Alignment::Center => AlignSelf::Center,
        Alignment::End => AlignSelf::End,
    }
}

pub(super) fn tracks<T>(sizes: &[TrackSize]) -> Result<Vec<T>, Error>
where
    T: TaffyMinContent + TaffyMaxContent + FromLength + FromFr,
{
    let mut out = Vec::with_capacity(sizes.len());
    for size in sizes {
        out.push(match *size {
            TrackSize::MinContent => T::MIN_CONTENT,
            TrackSize::MaxContent => T::MAX_CONTENT,
            TrackSize::Points(value) => {
                finite_positive(value)?;
                T::from_length(value)
            }
            TrackSize::Fraction(value) => {
                finite_number(value)?;
                if value < 0.0 {
                    return Err(Error::BadNumber);
                }
                T::from_fr(value)
            }
        });
    }
    Ok(out)
}

fn justify_content(justify: Justify) -> JustifyContent {
    match justify {
        Justify::Start => JustifyContent::Start,
        Justify::Center => JustifyContent::Center,
        Justify::End => JustifyContent::End,
        Justify::SpaceBetween => JustifyContent::SpaceBetween,
        Justify::SpaceAround => JustifyContent::SpaceAround,
        Justify::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

pub(super) fn scroll_style() -> Style {
    Style {
        display: Display::Grid,
        grid_template_columns: vec![max_content()],
        grid_template_rows: vec![max_content()],
        align_self: Some(AlignSelf::Stretch),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        overflow: taffy::Point {
            x: taffy::style::Overflow::Hidden,
            y: taffy::style::Overflow::Hidden,
        },
        ..Style::default()
    }
}

pub(super) fn stack_style() -> Style {
    Style {
        display: Display::Grid,
        grid_template_columns: vec![min_content()],
        grid_template_rows: vec![min_content()],
        ..Style::default()
    }
}

pub(super) fn flex_style(direction: FlexDirection, gap: f32) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: direction,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        align_items: Some(AlignItems::Stretch),
        justify_content: Some(JustifyContent::Start),
        ..Style::default()
    }
}

/// Size already laid out for this text, when `max_width` cannot change the lines.
///
/// Taffy asks once at max-content and again at the definite width. The max-content
/// pass uses a million-point constraint, which misses the paint slot and builds a
/// paragraph that is thrown away. A result that already fits inside its old
/// constraint is the same at any wider limit, including that probe.
fn reusable_text_size(
    text: &super::node::TextPayload,
    font_size: f32,
    max_width: f32,
) -> Option<TextSize> {
    let cached = text.cached.as_ref()?;
    let metrics = text.metrics.as_ref()?;
    let key = &cached.metrics;
    if key.font_size != font_size.to_bits() || key.text.as_ref() != text.utf8.as_ref() {
        return None;
    }
    if key.style.as_ref() != text.style.as_ref() || cached.color != text.color {
        return None;
    }
    if !spans_match(text, cached) {
        return None;
    }
    let old = f32::from_bits(key.max_width);
    let size = metrics.size;
    if key.max_width == max_width.to_bits() {
        return Some(size);
    }
    if size.width + 0.5 < old && size.width <= max_width + 0.5 {
        return Some(size);
    }
    None
}

fn spans_match(text: &super::node::TextPayload, cached: &crate::PaintKey) -> bool {
    if text.spans.len() != cached.metrics.spans.len()
        || text.spans.len() != cached.span_colors.len()
    {
        return false;
    }
    text.spans
        .iter()
        .zip(cached.metrics.spans.iter())
        .zip(cached.span_colors.iter())
        .all(|((span, metric), color)| {
            span.utf8 == metric.utf8.as_ref()
                && span.weight == metric.weight
                && span.underline == metric.underline
                && span.line_through == metric.line_through
                && span.family.as_deref().unwrap_or("") == metric.family.as_ref()
                && span.color == *color
        })
}

fn line_limit(space: AvailableSpace) -> f32 {
    match space {
        AvailableSpace::Definite(width) if width > 0.0 => width,
        AvailableSpace::Definite(_) => 0.0,
        AvailableSpace::MinContent | AvailableSpace::MaxContent => SINGLE_LINE_MAX,
    }
}

struct MeasureEnv<'a> {
    slots: &'a [Slot],
    shaper: &'a dyn Shaper,
    text_scale: f32,
    text_reused: &'a Cell<u32>,
    text_shaped: &'a Cell<u32>,
}

fn measure_leaf(
    env: &MeasureEnv<'_>,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut usize>,
    measured: &mut Vec<NodeId>,
) -> Size<f32> {
    let Some(index) = context.copied() else {
        return Size::ZERO;
    };
    let Some(slot) = env.slots.get(index) else {
        return Size::ZERO;
    };
    let Some(node) = slot.node.as_ref() else {
        return Size::ZERO;
    };
    let size = if let Some(text) = node.text() {
        // A min-content query sizes to the widest unbreakable word, not to
        // the single-line maximum: a grid min-content track and a scroll's
        // content floor use this to keep long words visible.
        if matches!(available.width, AvailableSpace::MinContent) {
            let font_size = text.font_size * env.text_scale;
            TextSize {
                width: env.shaper.min_content_width(&text.utf8, font_size),
                height: env
                    .shaper
                    .measure_paint(&crate::paint_key(
                        Arc::clone(&text.utf8),
                        Arc::clone(&text.style),
                        text.spans.as_ref(),
                        font_size,
                        f32::MAX,
                        text.color,
                    ))
                    .height,
            }
        } else {
            let max_width = line_limit(available.width);
            let font_size = text.font_size * env.text_scale;
            if let Some(size) = reusable_text_size(text, font_size, max_width) {
                env.text_reused.set(env.text_reused.get().saturating_add(1));
                let paints_at = text
                    .cached
                    .as_ref()
                    .map(|key| key.metrics.max_width)
                    .unwrap_or(0);
                if paints_at == max_width.to_bits() {
                    text.max_width.set(max_width);
                }
                size
            } else {
                let previous = text.max_width.get();
                text.max_width.set(max_width);
                if previous != max_width {
                    measured.push(NodeId {
                        index: crate::cast::u32_from_usize(index),
                        generation: slot.generation,
                    });
                }
                let key = crate::paint_key(
                    Arc::clone(&text.utf8),
                    Arc::clone(&text.style),
                    text.spans.as_ref(),
                    font_size,
                    max_width,
                    text.color,
                );
                env.text_shaped.set(env.text_shaped.get().saturating_add(1));
                env.shaper.measure_paint(&key)
            }
        }
    } else if let Some(image) = node.image() {
        TextSize {
            width: image.width,
            height: image.height,
        }
    } else {
        TextSize {
            width: 0.0,
            height: 0.0,
        }
    };
    Size {
        width: known.width.unwrap_or(size.width),
        height: known.height.unwrap_or(size.height),
    }
}

#[cfg(test)]
fn child_ids(tree: &Tree, id: NodeId) -> Vec<NodeId> {
    let mut ids = Vec::new();
    let mut child = tree.node(id).ok().and_then(|node| node.first_child);
    while let Some(current) = child {
        ids.push(current);
        child = tree.node(current).ok().and_then(|node| node.next);
    }
    ids
}

#[cfg(test)]
fn mark_descendants_dirty(tree: &mut Tree, id: NodeId) {
    if let Ok(node) = tree.node_mut(id) {
        node.flags.insert(NodeFlags::LAYOUT_DIRTY);
    }
    for child in child_ids(tree, id) {
        mark_descendants_dirty(tree, child);
    }
}

#[test]
fn short_line_reuses_a_wider_constraint() {
    let mut text = super::node::TextPayload::new("Item 1", 16.0, [0, 0, 0, 255]);
    text.cached = Some(crate::paint_key(
        Arc::clone(&text.utf8),
        Arc::clone(&text.style),
        &[],
        16.0,
        200.0,
        text.color,
    ));
    text.metrics = Some(Arc::new(crate::ParagraphMetrics::with_baseline(
        TextSize {
            width: 48.0,
            height: 16.0,
        },
        Arc::from([]),
        12.0,
    )));
    let exact = reusable_text_size(&text, 16.0, 200.0).expect("same width");
    assert_eq!(exact.width, 48.0);
    let wider = reusable_text_size(&text, 16.0, SINGLE_LINE_MAX).expect("max content");
    assert_eq!(wider.width, 48.0);
    assert!(reusable_text_size(&text, 16.0, 20.0).is_none());

    text.metrics = Some(Arc::new(crate::ParagraphMetrics::with_baseline(
        TextSize {
            width: 200.0,
            height: 32.0,
        },
        Arc::from([]),
        12.0,
    )));
    assert!(reusable_text_size(&text, 16.0, SINGLE_LINE_MAX).is_none());
    assert!(reusable_text_size(&text, 16.0, 200.0).is_some());
}

#[test]
fn relayout_boundary_moves_siblings_and_skips_a_clean_scroll() -> Result<(), Error> {
    let mut tree = Tree::new(crate::MonoShaper::default());
    tree.set_viewport(0.0, 0.0, 400.0, 200.0)?;
    let row = tree.row(0.0)?;
    let left = tree.text("A", 16.0, [0, 0, 0, 255])?;
    let middle = tree.text("B", 16.0, [0, 0, 0, 255])?;
    let right = tree.text("C", 16.0, [0, 0, 0, 255])?;
    let scroll = tree.scroll()?;
    let inner = tree.text("Inner", 16.0, [0, 0, 0, 255])?;
    tree.add_child(row, left)?;
    tree.add_child(row, middle)?;
    tree.add_child(row, right)?;
    tree.add_child(scroll, inner)?;
    tree.add_child(row, scroll)?;
    tree.layout()?;

    let left_scene = tree.rect(left)?;
    let right_before = tree.rect(right)?.x;
    tree.node_mut(inner)?.x = 12_345.0;
    tree.set_text(middle, "BBBB")?;
    tree.layout()?;

    assert_eq!(tree.node(inner)?.x, 12_345.0);
    assert_eq!(tree.rect(left)?.x, left_scene.x);
    assert!(tree.rect(right)?.x > right_before);

    let left_local = tree.node(left)?.x;
    let right_local = tree.node(right)?.x;
    mark_descendants_dirty(&mut tree, row);
    tree.place_boundary(row)?;
    assert_eq!(tree.node(left)?.x, left_local);
    assert_eq!(tree.node(right)?.x, right_local);
    let (scene_x, scene_y) = tree.layout_point(left)?;
    assert_eq!((tree.rect(left)?.x, tree.rect(left)?.y), (scene_x, scene_y));
    Ok(())
}
