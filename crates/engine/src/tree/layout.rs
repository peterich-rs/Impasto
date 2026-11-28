//! Taffy styles, measure, and the viewport.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use taffy::prelude::*;
use taffy::style::{ExpandedDimension, ExpandedLengthPercentage, ExpandedLengthPercentageAuto};

use super::node::{Kind, NodeFlags, Slot, SubtreeSpace};
use super::{Alignment, AxisSize, Edges, GridLine, Inset, Justify, NodeId, Rect, TrackSize, Tree};
use crate::{
    Error,
    text::{SINGLE_LINE_MAX, Shaper, TextSize},
};

pub(super) use crate::style::defaults::{
    content_box, fill_track, flex_style, scroll_style_for, stack_style,
};

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
        let next_width = width.max(0.0);
        let next_height = height.max(0.0);
        let window_changed = self.style_env.window_width != next_width
            || self.style_env.window_height != next_height;
        self.style_env.window_width = next_width;
        self.style_env.window_height = next_height;
        if window_changed {
            self.invalidate_style_deps(crate::style::DepMask::WINDOW);
        }
        self.note_layout(LayoutCause::Viewport);
        if let Some(root) = self.root {
            if let Some(taffy) = self.node(root)?.taffy {
                self.taffy.dirty_root(taffy, "viewport");
            }
            self.mark_layout_from(root);
            self.dirty_every_picture();
        } else {
            self.mark_layout();
        }
        Ok(())
    }

    /// Always-on safe area and the full window, in scene points.
    ///
    /// `place_panel` clamps to this rect. Keyboard insets are not included.
    pub fn set_safe_area(
        &mut self,
        window_w: f32,
        window_h: f32,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> Result<(), Error> {
        finite_number(window_w)?;
        finite_number(window_h)?;
        finite_number(left)?;
        finite_number(top)?;
        finite_number(right)?;
        finite_number(bottom)?;
        let window = (window_w.max(0.0), window_h.max(0.0));
        if self.window != window {
            self.window = window;
            self.canvas_geometry_changed();
        }
        self.safe = (left.max(0.0), top.max(0.0), right.max(0.0), bottom.max(0.0));
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
        self.needs_paint || !self.style_queue.is_empty()
    }

    pub fn layout_passes(&self) -> u64 {
        self.layout_passes
    }

    pub(super) fn note_layout(&mut self, cause: LayoutCause) {
        if self.layout_cause == LayoutCause::None {
            self.layout_cause = cause;
        }
        if self.pending_layout_cause == LayoutCause::None {
            self.pending_layout_cause = cause;
        }
    }

    pub fn trace_text_probes(&mut self, enabled: bool) {
        self.trace_text_probes = enabled;
    }

    /// Layout counters since the previous take. The next frame starts at zero.
    pub fn take_layout_trace(&mut self) -> LayoutTrace {
        let (cache_hits, cache_misses) = self.taffy.take_cache_counts();
        let trace = LayoutTrace {
            laywork_ms: self.layout_work_ms,
            passes: self.layout_entries,
            root_layouts: self.root_layouts,
            local_layouts: self.local_layouts,
            same_text_layouts: self.same_text_layouts,
            text_reused: self.text_reused.take(),
            text_shaped: self.text_shaped.take(),
            text_probes: std::mem::take(&mut self.text_probes),
            cause: self.layout_cause,
            root_trigger: self.root_layout_trigger.take(),
            cache_hits,
            cache_misses,
        };
        self.layout_work_ms = 0.0;
        self.layout_entries = 0;
        self.root_layouts = 0;
        self.local_layouts = 0;
        self.same_text_layouts = 0;
        self.layout_cause = LayoutCause::None;
        trace
    }

    pub(super) fn mark_layout(&mut self) {
        self.needs_layout = true;
        self.mark_paint();
    }

    pub(super) fn mark_paint(&mut self) {
        self.invalidate_layer_delta();
        self.needs_paint = true;
    }

    /// Diries the picture that owns `id`. A layer also dirties its parent picture.
    pub(super) fn mark_repaint(&mut self, id: NodeId) {
        self.stage_repaint(id, super::RepaintReason::STYLE_PAINT);
        let owner = self.dirty_picture(id);
        if self.renders_layer_node(id) {
            if let Some(parent) = self.get(id).and_then(|node| node.parent) {
                self.dirty_picture(parent);
            }
        } else if owner.is_some_and(|owner| self.queue_picture_patch(owner)) {
            return;
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    /// A layer property changed and the run commands did not.
    pub(super) fn paint_layer_value(&mut self) {
        self.canvas_geometry_changed();
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    /// The open-layer bracket flipped. Dirty every ancestor so outer runs rebuild.
    ///
    /// A clip's layer bit depends on descendants, so its summary is invalidated.
    pub(super) fn repaint_ancestors(&mut self, id: NodeId) {
        self.canvas_geometry_changed();
        self.invalidate_subtree_layers(id);
        let mut current = Some(id);
        while let Some(node_id) = current {
            self.stage_repaint(node_id, super::RepaintReason::ANCESTOR_INVALIDATED);
            self.mark_picture_dirty(node_id);
            current = self.get(node_id).and_then(|node| node.parent);
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    pub(super) fn dirty_picture(&mut self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let parent = self.node(id).ok()?.parent;
            if parent.is_none() || self.renders_layer_node(id) || self.is_picture_owner(id) {
                self.mark_picture_dirty(id);
                return Some(id);
            }
            id = parent?;
        }
    }

    pub(super) fn is_picture_owner(&self, id: NodeId) -> bool {
        self.get(id).is_some_and(|node| {
            node.flags.intersects(
                NodeFlags::PICTURE_OWNER
                    | NodeFlags::PICTURE_FROM_CANVAS
                    | NodeFlags::PICTURE_FROM_SHADER
                    | NodeFlags::PICTURE_FROM_BOUNDARY,
            )
        })
    }

    /// Records the subtree of `id` as its own picture. Toggles only the list-row
    /// bit, so a canvas on the same node stays installed. The first transition
    /// dirties `id` and its parent owner so the parent run drops the old commands.
    /// A repeat call does not.
    pub(crate) fn set_picture_owner(&mut self, id: NodeId, on: bool) -> Result<(), Error> {
        self.set_picture_flag(id, NodeFlags::PICTURE_OWNER, on)
    }

    /// Records this node's independent picture for application-driven repaints.
    pub fn set_repaint_boundary(&mut self, id: NodeId, on: bool) -> Result<(), Error> {
        self.set_picture_flag(id, NodeFlags::PICTURE_FROM_BOUNDARY, on)
    }

    /// Allows the rasterizer to cache this node's subtree during its transitions.
    pub fn set_transition_snapshot(&mut self, id: NodeId, on: bool) -> Result<(), Error> {
        if self
            .node(id)?
            .flags
            .contains(NodeFlags::TRANSITION_SNAPSHOT)
            != on
        {
            self.node_mut(id)?
                .flags
                .set(NodeFlags::TRANSITION_SNAPSHOT, on);
            self.invalidate_subtree_layers(id);
            self.layer_epoch = self.layer_epoch.wrapping_add(1);
            self.mark_paint();
        }
        Ok(())
    }

    fn set_picture_flag(&mut self, id: NodeId, flag: NodeFlags, on: bool) -> Result<(), Error> {
        if self.node(id)?.flags.contains(flag) == on {
            return Ok(());
        }
        self.node_mut(id)?.flags.set(flag, on);
        self.invalidate_subtree_layers(id);
        self.dirty_picture(id);
        if let Some(parent) = self.get(id).and_then(|node| node.parent) {
            self.dirty_picture(parent);
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
        Ok(())
    }

    /// Installs or clears the canvas on `id`.
    ///
    /// An empty op list still owns a picture. Repaint only; layout stays put.
    /// Sources in the list join [`Tree::image_census`] until the canvas is cleared
    /// or the node is retired. The list-row picture bit is left alone.
    pub fn set_canvas(
        &mut self,
        id: NodeId,
        list: Option<crate::scene::CanvasList>,
    ) -> Result<(), Error> {
        let was_owner = self.is_picture_owner(id);
        let previous = self.node_mut(id)?.canvas.take();
        if previous.is_none() && list.is_none() {
            return Ok(());
        }
        if let Some(previous) = previous.as_ref() {
            self.note_canvas_images(previous, -1);
        }
        if let Some(next) = list.as_ref() {
            self.note_canvas_images(next, 1);
        }
        {
            let node = self.node_mut(id)?;
            match list {
                Some(next) => {
                    node.flags.insert(NodeFlags::PICTURE_FROM_CANVAS);
                    node.canvas = Some(next);
                }
                None => node.flags.remove(NodeFlags::PICTURE_FROM_CANVAS),
            }
        }
        self.stage_repaint(id, super::RepaintReason::CANVAS_CALLBACK);
        self.repaint_picture_owner(id, was_owner);
        Ok(())
    }

    pub(super) fn repaint_picture_owner(&mut self, id: NodeId, was_owner: bool) {
        if was_owner == self.is_picture_owner(id) {
            self.mark_repaint(id);
        } else {
            self.invalidate_subtree_layers(id);
            self.dirty_picture(id);
            if let Some(parent) = self.get(id).and_then(|node| node.parent) {
                self.dirty_picture(parent);
            }
            self.layer_epoch = self.layer_epoch.wrapping_add(1);
            self.mark_paint();
        }
    }

    pub(super) fn renders_layer_node(&self, id: NodeId) -> bool {
        let subtree = self
            .subtree_layers
            .get(&id)
            .is_some_and(|value| value.requires_clip_layer());
        self.get(id).is_some_and(|node| {
            node.renders_layers(subtree) || node.flags.contains(NodeFlags::PICTURE_FROM_BOUNDARY)
        }) || self.uses_paint_slot(id)
            || self.motion_layers.get(&id).is_some_and(|layers| {
                layers.linked_transform || layers.transition_opacity || layers.linked_effect
            })
    }

    pub(super) fn is_repaint_dirty(&self, id: NodeId) -> bool {
        self.get(id)
            .is_some_and(|node| node.flags.contains(NodeFlags::REPAINT_DIRTY))
    }

    fn mark_picture_dirty(&mut self, id: NodeId) {
        let Ok(node) = self.node_mut(id) else { return };
        let starts_repaint = node.flags.contains(NodeFlags::PICTURE_FROM_BOUNDARY)
            && !node.flags.contains(NodeFlags::REPAINT_DIRTY);
        node.flags.insert(NodeFlags::REPAINT_DIRTY);
        if starts_repaint {
            self.repaint_boundaries_dirtied = self.repaint_boundaries_dirtied.saturating_add(1);
        }
    }

    #[cfg(feature = "js")]
    pub(crate) fn take_repaint_boundaries_dirtied(&mut self) -> u32 {
        std::mem::take(&mut self.repaint_boundaries_dirtied)
    }

    pub(super) fn dirty_every_picture(&mut self) {
        let Some(root) = self.root else {
            return;
        };
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
            self.mark_picture_dirty(id);
        }
    }

    pub fn clear_paint(&mut self) {
        self.needs_paint = false;
    }

    /// The foreground of text or the fill of a decorated box.
    pub fn color(&self, id: NodeId) -> Result<Option<[u8; 4]>, Error> {
        let node = self.node(id)?;
        Ok(node.text().map(|text| text.color).or_else(|| node.fill()))
    }

    /// Foreground for text, fill for a decorated box.
    pub fn set_color(&mut self, id: NodeId, color: [u8; 4]) -> Result<(), Error> {
        if self.node(id)?.is_text() {
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
            // Color is paint. The measure key ignores it.
            self.mark_repaint(id);
            return Ok(());
        }
        if let Some(previous) = self.node(id)?.fill() {
            if previous == color {
                return Ok(());
            }
            self.node_mut(id)?
                .write_paint(|paint| paint.fill = Some(color));
            if !self.update_rect_slot(id) {
                self.mark_repaint(id);
            }
            return Ok(());
        }
        Err(Error::WrongKind)
    }

    pub fn set_gap(&mut self, id: NodeId, gap: f32) -> Result<(), Error> {
        finite_number(gap)?;
        if !self.node(id)?.is_flex_line() || !self.node(id)?.is_box() {
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

    /// Padding on a container. A non-finite edge leaves the style unchanged.
    pub fn set_padding_edges(&mut self, id: NodeId, edges: Edges) -> Result<(), Error> {
        let padding = edge_rect(edges)?;
        if !self.node(id)?.is_box() {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.padding = padding;
        self.write_style(id, style)
    }

    /// Replaces one padding edge. The other three stay as stored on the node.
    pub fn set_padding_edge(
        &mut self,
        id: NodeId,
        edge: super::BoxEdge,
        value: f32,
    ) -> Result<(), Error> {
        finite_number(value)?;
        let current = self.node(id)?.style.padding;
        let mut edges = Edges {
            top: points_of(current.top),
            right: points_of(current.right),
            bottom: points_of(current.bottom),
            left: points_of(current.left),
        };
        match edge {
            super::BoxEdge::Top => edges.top = value,
            super::BoxEdge::Right => edges.right = value,
            super::BoxEdge::Bottom => edges.bottom = value,
            super::BoxEdge::Left => edges.left = value,
        }
        self.set_padding_edges(id, edges)
    }

    /// Replaces one inset edge. `None` is auto.
    pub fn set_inset_edge(
        &mut self,
        id: NodeId,
        edge: super::BoxEdge,
        value: Option<f32>,
    ) -> Result<(), Error> {
        let current = self.node(id)?.style.inset;
        let mut inset = Inset {
            top: auto_points(current.top),
            right: auto_points(current.right),
            bottom: auto_points(current.bottom),
            left: auto_points(current.left),
        };
        match edge {
            super::BoxEdge::Top => inset.top = value,
            super::BoxEdge::Right => inset.right = value,
            super::BoxEdge::Bottom => inset.bottom = value,
            super::BoxEdge::Left => inset.left = value,
        }
        self.set_inset(id, inset)
    }

    /// Replaces one transform component. Scale stays uniform.
    pub fn set_transform_component(
        &mut self,
        id: NodeId,
        component: u8,
        value: f32,
    ) -> Result<(), Error> {
        let current = self.node(id)?.transform();
        let (x, y, rotate, scale) = match component {
            0 => (
                value,
                current.translate_y,
                current.rotate_rad,
                current.scale_x,
            ),
            1 => (
                current.translate_x,
                value,
                current.rotate_rad,
                current.scale_x,
            ),
            2 => (
                current.translate_x,
                current.translate_y,
                value,
                current.scale_x,
            ),
            _ => (
                current.translate_x,
                current.translate_y,
                current.rotate_rad,
                value,
            ),
        };
        self.set_transform(id, x, y, rotate, scale)
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
        self.write_style(id, style)?;
        Ok(())
    }

    pub fn set_max_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.max_size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)?;
        Ok(())
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
        if !node.is_box() {
            return Err(Error::WrongKind);
        }
        node.write_paint(|paint| paint.corner_radius = radius);
        let _ = node;
        if !self.update_rect_slot(id) && !self.update_clip_radius(id) {
            self.mark_repaint(id);
        }
        Ok(())
    }

    /// When `cull` is set, record and hit testing skip this subtree.
    /// The laid-out box does not change.
    pub fn set_cull_paint(&mut self, id: NodeId, cull: bool) -> Result<(), Error> {
        {
            let node = self.node_mut(id)?;
            if node.flags.contains(NodeFlags::CULL_PAINT) == cull {
                return Ok(());
            }
            node.flags.set(NodeFlags::CULL_PAINT, cull);
        }
        self.dirty_picture(id);
        self.mark_paint();
        Ok(())
    }

    /// Alpha of an opacity container, 0–255. Does not invalidate layout.
    pub fn set_alpha(&mut self, id: NodeId, alpha: u8) -> Result<(), Error> {
        if self.node(id)?.alpha().is_none() {
            return Err(Error::WrongKind);
        }
        self.write_alpha(id, alpha)
    }

    fn write_alpha(&mut self, id: NodeId, alpha: u8) -> Result<(), Error> {
        if self.node(id)?.alpha().unwrap_or(255) == alpha {
            return Ok(());
        }
        let previous = alpha_bucket(self.node(id)?.alpha());
        self.node_mut(id)?
            .write_paint(|paint| paint.alpha = Some(alpha));
        let next = alpha_bucket(self.node(id)?.alpha());
        if previous == next {
            if !self.update_opacity_slot(id, alpha) {
                self.paint_layer_value();
            }
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    pub fn set_align(&mut self, id: NodeId, alignment: Alignment) -> Result<(), Error> {
        if !self.node(id)?.is_box() {
            return Err(Error::WrongKind);
        }
        let (cross, main) = align_axes(alignment);
        let mut style = self.node(id)?.style.clone();
        style.align_items = cross;
        style.justify_content = main;
        self.write_style(id, style)
    }

    /// `true` writes the reverse direction of a row or a column.
    pub fn set_reverse(&mut self, id: NodeId, reverse: bool) -> Result<(), Error> {
        let node = self.node(id)?;
        if !node.is_box() || !node.is_flex_line() {
            return Err(Error::WrongKind);
        }
        let column = node.is_column_flex();
        let mut style = node.style.clone();
        style.flex_direction = match (column, reverse) {
            (true, true) => FlexDirection::ColumnReverse,
            (true, false) => FlexDirection::Column,
            (false, true) => FlexDirection::RowReverse,
            (false, false) => FlexDirection::Row,
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
    /// [`crate::tree::RowCrossAlign::Baseline`] on a column returns `Ok` and leaves the style
    /// alone. [`crate::tree::RowCrossAlign::Stretch`] writes `AlignItems::STRETCH`.
    pub fn set_row_cross(&mut self, id: NodeId, align: super::RowCrossAlign) -> Result<(), Error> {
        let node = self.node(id)?;
        if !node.is_box() || !node.is_flex_line() {
            return Err(Error::WrongKind);
        }
        if node.is_column_flex() && align == super::RowCrossAlign::Baseline {
            return Ok(());
        }
        let cross = match align {
            super::RowCrossAlign::Start => AlignItems::START,
            super::RowCrossAlign::Baseline => AlignItems::BASELINE,
            super::RowCrossAlign::Center => AlignItems::CENTER,
            super::RowCrossAlign::End => AlignItems::END,
            super::RowCrossAlign::Stretch => AlignItems::STRETCH,
        };
        let mut style = self.node(id)?.style.clone();
        style.align_items = cross;
        self.write_style(id, style)
    }

    /// Main-axis alignment of a row or column.
    pub fn set_justify(&mut self, id: NodeId, justify: Justify) -> Result<(), Error> {
        if !self.node(id)?.is_box() || !self.node(id)?.is_flex_line() {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.justify_content = justify_content(justify);
        self.write_style(id, style)
    }

    /// `true` lets a row or column wrap onto another line.
    pub fn set_wrap(&mut self, id: NodeId, wrap: bool) -> Result<(), Error> {
        if !self.node(id)?.is_box() || !self.node(id)?.is_flex_line() {
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
    /// Minimum size is zero and both flex factors are one. A row or column can
    /// shrink this node to the space left after siblings, which is what keeps a
    /// tab bar on screen. The factors are stored even when `fill` runs before
    /// insert: JSX sets the prop at creation. An indefinite parent treats the
    /// percentage as auto, so the node still sizes to its content. A filled
    /// stack, clip, opacity, or transform uses `minmax(0, 1fr)`. A plain `1fr`
    /// track is `minmax(auto, 1fr)` and grows with the child past the box.
    pub fn set_fill(&mut self, id: NodeId) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        };
        style.min_size = Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        };
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
        if self.node(id)?.is_stack() {
            style.grid_template_columns = vec![GridTemplateComponent::Single(fill_track())];
            style.grid_template_rows = vec![GridTemplateComponent::Single(fill_track())];
        }
        self.write_style(id, style)
    }

    pub fn set_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        if !self.node(id)?.is_box() {
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

    /// Replaces one size axis. The other axis stays as stored on the node.
    pub fn set_size_axis(&mut self, id: NodeId, axis: u8, size: AxisSize) -> Result<(), Error> {
        let current = self.node(id)?.style.size;
        let width = if axis == 0 {
            size
        } else {
            axis_size_of(current.width)
        };
        let height = if axis == 0 {
            axis_size_of(current.height)
        } else {
            size
        };
        self.set_axis_size(id, width, height)
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

    /// Places a child of an author grid. Stack-like grids return [`Error::WrongKind`].
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
        if !self.node(parent)?.is_author_grid() {
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

    /// Replaces one grid line. The other three stay as stored on the node.
    pub fn set_grid_line(&mut self, id: NodeId, edge: u8, line: GridLine) -> Result<(), Error> {
        let style = self.node(id)?.style.clone();
        let mut column_start = grid_line_of(style.grid_column.start);
        let mut column_end = grid_line_of(style.grid_column.end);
        let mut row_start = grid_line_of(style.grid_row.start);
        let mut row_end = grid_line_of(style.grid_row.end);
        match edge {
            0 => column_start = line,
            1 => column_end = line,
            2 => row_start = line,
            _ => row_end = line,
        }
        self.set_grid_placement(id, column_start, column_end, row_start, row_end)
    }

    /// Replace the image resource. `intrinsic` is the measure fallback only.
    ///
    /// `None` leaves the stored intrinsic size alone, including the zero
    /// placeholder used before a resource is known. Author width and height
    /// stay on the style sources.
    pub fn set_image(
        &mut self,
        id: NodeId,
        source: &str,
        intrinsic: Option<(f32, f32)>,
    ) -> Result<(), Error> {
        if let Some((width, height)) = intrinsic {
            let _ = finite_positive(width)?;
            let _ = finite_positive(height)?;
        }
        let previous = {
            let node = self.node_mut(id)?;
            let Some(image) = node.image_mut() else {
                return Err(Error::WrongKind);
            };
            let previous = Arc::clone(&image.source);
            image.source = Arc::from(source);
            if let Some((width, height)) = intrinsic {
                image.width = width;
                image.height = height;
            }
            previous
        };
        if previous.as_ref() != source {
            self.note_image(&previous, -1);
            self.note_image(&Arc::from(source), 1);
        }
        self.mark_repaint(id);
        self.touch_measure(id)
    }

    /// Fit mode and whether a missing decode fails the frame. The source string stays `asset:name`.
    pub fn set_image_mode(
        &mut self,
        id: NodeId,
        mode: crate::scene::ContentMode,
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
                crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY => 0.0,
                crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY => {
                    let (max_x, max_y) = self.scroll_limits(motion.node);
                    let limit = if motion.prop == crate::motion::AnimProp::ScrollX {
                        max_x
                    } else {
                        max_y
                    };
                    motion.simulation.terminal().clamp(0.0, limit.max(0.0))
                }
                _ => motion.simulation.terminal(),
            };
            let _ = self.apply_motion(motion.node, motion.prop, value);
            self.settle_style_motion(motion.node, motion.prop, value);
        }
    }

    /// A spring, coast, or overscroll return is still sampling.
    ///
    /// The session asks for another frame while this is true. Scroll coast is
    /// not the only motion that has to keep moving on a static page.
    pub fn motion_active(&self) -> bool {
        !self.motions.is_empty()
    }

    /// Current sample of the motion on this property, if it is still running.
    pub fn motion_position(&self, id: NodeId, prop: crate::motion::AnimProp) -> Option<f32> {
        let motion = self
            .motions
            .iter()
            .find(|motion| motion.node == id && motion.prop == prop)?;
        let time_ms = (self.clock_ms - motion.started_ms).max(0.0);
        Some(motion.simulation.x(time_ms))
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
            let mut value = self.motions[index].simulation.x(t);
            let mut finished = self.motions[index].simulation.done(t);
            let velocity = self.motions[index].simulation.dx(t);
            let node = self.motions[index].node;
            let prop = self.motions[index].prop;
            let snap = self.motions[index].snap;
            if matches!(
                prop,
                crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY
            ) {
                let previous = self
                    .node(node)
                    .ok()
                    .map(|sampled| sampled.overscroll())
                    .unwrap_or((0.0, 0.0));
                let previous = if prop == crate::motion::AnimProp::OverscrollX {
                    previous.0
                } else {
                    previous.1
                };
                if previous * value < 0.0 {
                    value = 0.0;
                    finished = true;
                }
            }
            let stop = self.apply_motion(node, prop, value) || finished;
            if stop {
                self.settle_style_motion(node, prop, value);
                self.motion_ends.push((node, prop));
                self.motions.remove(index);
                if matches!(
                    prop,
                    crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY
                ) {
                    self.settle_scroll_pixels(node);
                    self.after_scroll_motion(node, prop, velocity, snap);
                } else if matches!(
                    prop,
                    crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY
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
    pub fn take_motion_ends(&mut self) -> Vec<(NodeId, crate::motion::AnimProp)> {
        std::mem::take(&mut self.motion_ends)
    }

    /// Replaces any motion of `prop` on `id`. The sample clock is the last tick.
    pub fn animate(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        simulation: Box<dyn crate::motion::Simulation>,
    ) {
        self.motions
            .retain(|motion| !(motion.node == id && motion.prop == prop));
        self.next_motion_id = self.next_motion_id.wrapping_add(1);
        self.motions.push(super::ActiveMotion {
            id: crate::scene::CompositorTrackId(self.next_motion_id),
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
        prop: crate::motion::AnimProp,
        simulation: Box<dyn crate::motion::Simulation>,
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

    pub(super) fn apply_motion(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
        value: f32,
    ) -> bool {
        self.applying_motion = true;
        let stop = match prop {
            crate::motion::AnimProp::ScrollX | crate::motion::AnimProp::ScrollY => {
                self.apply_scroll(id, prop, value)
            }
            crate::motion::AnimProp::Opacity => {
                let alpha = crate::scene::compositor_alpha(value);
                self.write_alpha(id, alpha).is_err()
            }
            crate::motion::AnimProp::TranslateX
            | crate::motion::AnimProp::TranslateY
            | crate::motion::AnimProp::Rotate
            | crate::motion::AnimProp::Scale => self.apply_transform_prop(id, prop, value),
            crate::motion::AnimProp::OverscrollX | crate::motion::AnimProp::OverscrollY => {
                self.apply_overscroll(id, prop, value)
            }
            crate::motion::AnimProp::ShaderUniform(index) => {
                self.apply_shader_uniform(id, index, value)
            }
        };
        self.applying_motion = false;
        stop
    }

    pub(super) fn apply_transform_prop(
        &mut self,
        id: NodeId,
        prop: crate::motion::AnimProp,
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
            crate::motion::AnimProp::TranslateX => translate_x = value,
            crate::motion::AnimProp::TranslateY => translate_y = value,
            crate::motion::AnimProp::Rotate => rotate = value,
            crate::motion::AnimProp::Scale => scale = value,
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
        if self.node(id)?.role.contains(super::node::Role::TRANSFORM) {
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
            if !self.update_transform_slot(id) {
                self.paint_layer_value();
            }
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    pub fn layout(&mut self) -> Result<(), Error> {
        self.flush_styles()?;
        if !self.needs_layout && self.laid_out {
            return Ok(());
        }
        let pass = self.layout_passes.saturating_add(1);
        let _trace = tracing::debug_span!(target: "impasto::layout", "layout_pass",
            pass, cause = self.pending_layout_cause.name())
        .entered();
        let root = self.root.ok_or(Error::NoRoot)?;
        let root_taffy = self.node(root)?.taffy.ok_or(Error::NoRoot)?;
        let available = match self.viewport {
            Some((width, height)) => Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            None => Size::MAX_CONTENT,
        };
        let started = Instant::now();
        let mut measured = Vec::new();
        self.open_measure();
        {
            let _span =
                tracing::trace_span!(target: "impasto::layout", "check_changed_text").entered();
            self.check_changed_text()?;
        }
        self.taffy.begin_layout();
        let taffy_started = Instant::now();
        let (root_trigger, local) = {
            let _span = tracing::debug_span!(target: "impasto::layout", "taffy_compute").entered();
            let mut env = MeasureEnv {
                slots: &self.slots,
                shaper: &mut *self.shaper,
                text_scale: self.text_scale,
                text_reused: &self.text_reused,
                text_shaped: &self.text_shaped,
                provisional: HashMap::new(),
            };
            let mut measure = |inputs, context, style: &Style| {
                leaf_output(inputs, style, |known, available| {
                    measure_leaf(&mut env, known, available, context, &mut measured)
                })
            };
            let (root_trigger, local) = self.taffy.relayout(!self.laid_out, &mut measure);
            if root_trigger.is_some() {
                self.taffy
                    .compute_layout_with_measure(root_taffy, available, &mut measure);
            }
            (root_trigger, local)
        };
        self.taffy_ms += taffy_started.elapsed().as_secs_f64() * 1000.0;
        for id in measured {
            self.note_text(id);
        }
        {
            let _span = tracing::trace_span!(target: "impasto::layout", "place_boundary").entered();
            if let Some(trigger) = root_trigger {
                self.place_boundary(root)?;
                self.root_layout_trigger.get_or_insert(trigger);
                self.root_layouts = self.root_layouts.saturating_add(1);
            } else {
                for &id in &local {
                    self.place_boundary(id)?;
                }
            }
        }
        self.local_layouts = self
            .local_layouts
            .saturating_add(crate::cast::u32_from_usize(local.len()));
        if root_trigger.is_some() || !local.is_empty() {
            self.layout_passes = self.layout_passes.saturating_add(1);
            self.layout_entries = self.layout_entries.saturating_add(1);
            self.canvas_geometry_changed();
        }
        self.needs_layout = false;
        self.mark_paint();
        self.laid_out = true;
        self.pending_layout_cause = LayoutCause::None;
        {
            let _span = tracing::trace_span!(target: "impasto::layout", "sync_metrics").entered();
            self.sync_metrics();
        }
        self.layout_work_ms += started.elapsed().as_secs_f64() * 1000.0;
        if self.debug_scan_enabled() {
            self.scan_overflow();
        }
        Ok(())
    }

    pub(super) fn queue_text_measure(&mut self, id: NodeId) {
        self.note_layout(LayoutCause::Text);
        self.note_text(id);
        if !self.changed_text.contains(&id) {
            self.changed_text.push(id);
        }
        self.mark_layout();
        self.mark_repaint(id);
    }

    fn check_changed_text(&mut self) -> Result<(), Error> {
        if self.changed_text.is_empty() {
            return Ok(());
        }
        let started = self.trace_text_probes.then(Instant::now);
        let before = started.map(|_| self.shaper.measure_cache_counts());
        let mut measurements = Vec::new();
        for id in std::mem::take(&mut self.changed_text) {
            let Some(slot) = self
                .slots
                .get(id.index as usize)
                .filter(|slot| slot.generation == id.generation)
            else {
                continue;
            };
            let Some(node) = &slot.node else {
                continue;
            };
            let Some(taffy) = node.taffy else {
                continue;
            };
            let reuse = if let Some(text) = node.text()
                && self.laid_out
                && self.taffy.can_probe_text(taffy)
            {
                let width = text.max_width.get();
                let mut env = MeasureEnv {
                    slots: &self.slots,
                    shaper: &mut *self.shaper,
                    text_scale: self.text_scale,
                    text_reused: &self.text_reused,
                    text_shaped: &self.text_shaped,
                    provisional: HashMap::new(),
                };
                measurements.clear();
                let trace = &mut self.text_probes;
                let same = self
                    .taffy
                    .text_outputs_match(taffy, |inputs, context, style| {
                        leaf_output(inputs, style, |known, available| {
                            let intrinsic = matches!(available.width, AvailableSpace::MinContent);
                            let key = (line_limit(available.width).to_bits(), intrinsic);
                            if started.is_some() {
                                trace.queries = trace.queries.saturating_add(1);
                                if intrinsic {
                                    trace.intrinsic = trace.intrinsic.saturating_add(1);
                                }
                            }
                            // Only share the raw measurement of this node at the exact
                            // width and mode. Taffy still applies each query's own box.
                            let (size, baseline) = if let Some((_, output)) =
                                measurements.iter().find(|(old, _)| *old == key)
                            {
                                if started.is_some() {
                                    trace.repeated = trace.repeated.saturating_add(1);
                                }
                                *output
                            } else {
                                let measure_started = started.map(|_| Instant::now());
                                env.provisional.clear();
                                let output = measure_leaf(
                                    &mut env,
                                    Size::NONE,
                                    available,
                                    context,
                                    &mut Vec::new(),
                                );
                                if let Some(started) = measure_started {
                                    trace.measure_time += started.elapsed();
                                }
                                measurements.push((key, output));
                                output
                            };
                            (
                                Size {
                                    width: known.width.unwrap_or(size.width),
                                    height: known.height.unwrap_or(size.height),
                                },
                                baseline,
                            )
                        })
                    });
                text.max_width.set(width);
                same
            } else {
                false
            };
            if reuse {
                self.same_text_layouts = self.same_text_layouts.saturating_add(1);
            } else {
                self.taffy.mark_dirty(taffy)?;
                self.mark_layout_from(id);
            }
        }
        if let (Some(started), Some((old_hits, old_misses))) = (started, before) {
            let (hits, misses) = self.shaper.measure_cache_counts();
            self.text_probes.check_time += started.elapsed();
            let trace = &mut self.text_probes;
            trace.cache_hits = trace
                .cache_hits
                .saturating_add(hits.saturating_sub(old_hits));
            trace.cache_misses = trace
                .cache_misses
                .saturating_add(misses.saturating_sub(old_misses));
        }
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

    pub(super) fn open_measure(&mut self) {
        if self.measure_open {
            return;
        }
        self.shaper.begin_frame();
        self.measure_open = true;
    }

    pub(super) fn finish_measure(&mut self) {
        if !self.measure_open {
            return;
        }
        self.shaper.end_frame();
        self.measure_open = false;
    }

    fn measure_subtree(
        &mut self,
        taffy_id: taffy::NodeId,
        width: AvailableSpace,
        height: AvailableSpace,
        measured: &mut Vec<NodeId>,
    ) -> Result<Size<f32>, Error> {
        let _trace = tracing::trace_span!(target: "impasto::layout", "measure_subtree").entered();
        let shaper = &mut *self.shaper;
        let mut env = MeasureEnv {
            slots: &self.slots,
            shaper,
            text_scale: self.text_scale,
            text_reused: &self.text_reused,
            text_shaped: &self.text_shaped,
            provisional: HashMap::new(),
        };
        self.taffy.compute_layout_with_measure(
            taffy_id,
            Size { width, height },
            |inputs, context, style| {
                leaf_output(inputs, style, |known, available| {
                    measure_leaf(&mut env, known, available, context, measured)
                })
            },
        );
        self.taffy.forget_parent_input(taffy_id);
        Ok(self.taffy.layout(taffy_id)?.size)
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
        let started = Instant::now();
        let mut measured = Vec::new();
        self.open_measure();
        let natural = self.measure_subtree(taffy_id, first_width, first_height, &mut measured)?;
        let mut size = natural;
        if let SubtreeSpace::CappedMax(cap) = space {
            if natural.width > cap {
                size = self.measure_subtree(
                    taffy_id,
                    AvailableSpace::Definite(cap),
                    AvailableSpace::MaxContent,
                    &mut measured,
                )?;
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
        self.canvas_geometry_changed();
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

    /// Measures `panel` and writes only its transform so it sits outside `anchor`.
    ///
    /// The anchor is read with [`Tree::scene_rect`]. Width is capped to the window
    /// minus the always-on safe area. A panel that does not fit below flips above.
    pub fn place_panel(
        &mut self,
        panel: NodeId,
        anchor: NodeId,
        align: super::PanelAlign,
    ) -> Result<(), Error> {
        let (safe_l, safe_t, safe_r, safe_b) = self.safe;
        let (win_w, win_h) = if self.window.0 > 0.0 && self.window.1 > 0.0 {
            self.window
        } else {
            let (vw, vh) = self.viewport.unwrap_or((0.0, 0.0));
            (self.origin.0 + vw, self.origin.1 + vh)
        };
        let cap = (win_w - safe_l - safe_r).max(1.0);
        let (_, ph) = self.measure_panel(panel, SubtreeSpace::CappedMax(cap))?;
        let pw = self.node(panel)?.w;
        let anchor_box = self.scene_rect(anchor)?;
        let (mut x, mut y) = match align {
            super::PanelAlign::BelowStart => (anchor_box.x, anchor_box.y + anchor_box.h),
            super::PanelAlign::BelowEnd => (
                anchor_box.x + anchor_box.w - pw,
                anchor_box.y + anchor_box.h,
            ),
            super::PanelAlign::BelowCenter => (
                anchor_box.x + (anchor_box.w - pw) * 0.5,
                anchor_box.y + anchor_box.h,
            ),
            super::PanelAlign::AboveStart => (anchor_box.x, anchor_box.y - ph),
        };
        let right = win_w - safe_r;
        let bottom = win_h - safe_b;
        if y + ph > bottom && anchor_box.y - ph >= safe_t {
            y = anchor_box.y - ph;
        }
        let max_x = (right - pw).max(safe_l);
        let max_y = (bottom - ph).max(safe_t);
        x = x.clamp(safe_l, max_x);
        y = y.clamp(safe_t, max_y);
        self.set_transform(panel, x, y, 0.0, 1.0)
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
                kind == Kind::Scroll || is_root || points, // scroll, root, or a definite box
            );
        }
        if let Some(taffy_id) = taffy_id {
            self.taffy.set_style(taffy_id, style)?;
            if is_root {
                self.taffy.dirty_root(taffy_id, "style");
            }
        }
        if !self.style_applying {
            let preset = self.node(id)?.style.clone();
            let generation = id.generation;
            if let Some(slot) = self.style_nodes.get_mut(&id.index)
                && slot.generation == generation
            {
                slot.preset = preset;
            }
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
            let flags = self.node(current)?.flags;
            let changed = self
                .node(current)?
                .taffy
                .is_some_and(|taffy| self.taffy.subtree_changed(taffy));
            let descend = changed
                || flags.intersects(NodeFlags::LAYOUT_DIRTY | NodeFlags::BOUNDARY_DIRTY)
                || !flags.contains(NodeFlags::RELAYOUT_BOUNDARY);
            if descend {
                self.place_boundary(current)?;
            } else {
                self.write_box_from_taffy(current)?;
            }
            child = next;
        }
        self.clear_layout_flags(id);
        Ok(())
    }

    pub(super) fn write_box_from_taffy(&mut self, id: NodeId) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy.ok_or(Error::UnknownNode)?;
        let (x, y, w, h, content_x, content_y, content_w, content_h) = {
            let layout = self.taffy.layout(taffy_id)?;
            let (content_x, content_y, content_w, content_h) = super::geometry::content_box(layout);
            (
                layout.location.x,
                layout.location.y,
                layout.size.width,
                layout.size.height,
                content_x,
                content_y,
                content_w,
                content_h,
            )
        };
        let node = self.node_mut(id)?;
        let changed = (node.x, node.y, node.w, node.h) != (x, y, w, h);
        node.x = x;
        node.y = y;
        node.w = w;
        node.h = h;
        node.content_x = content_x;
        node.content_y = content_y;
        node.content_w = content_w;
        node.content_h = content_h;
        if changed {
            self.stage_repaint(id, super::RepaintReason::LAYOUT_GEOMETRY);
        }
        Ok(())
    }

    pub(super) fn clear_layout_flags(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags
                .remove(NodeFlags::LAYOUT_DIRTY | NodeFlags::BOUNDARY_DIRTY);
        }
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
    matches!(size.expand(), ExpandedDimension::Length(_))
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
    pub root_layouts: u32,
    pub local_layouts: u32,
    pub same_text_layouts: u32,
    pub text_reused: u32,
    pub text_shaped: u32,
    pub text_probes: TextProbeTrace,
    pub cause: LayoutCause,
    pub root_trigger: Option<RootLayoutTrigger>,
    pub cache_hits: u32,
    pub cache_misses: u32,
}

/// Work spent replaying the saved leaf constraints of changed text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextProbeTrace {
    pub check_time: Duration,
    pub measure_time: Duration,
    pub queries: u32,
    pub intrinsic: u32,
    /// Queries with the same width and intrinsic mode as an earlier query of this node.
    pub repeated: u32,
    pub cache_hits: u32,
    pub cache_misses: u32,
}

/// The first invalidation that reached the root in one layout pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootLayoutTrigger {
    pub node: Option<NodeId>,
    pub cause: &'static str,
}

pub(super) fn finite_positive(value: f32) -> Result<f32, Error> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(Error::BadNumber)
    }
}

fn axis_dimension<T: TaffyAuto + FromLength + FromPercent>(size: AxisSize) -> Result<T, Error> {
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
        Alignment::Start => (AlignItems::START, JustifyContent::START),
        Alignment::Center => (AlignItems::CENTER, JustifyContent::CENTER),
        Alignment::End => (AlignItems::END, JustifyContent::END),
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

fn points_of(value: LengthPercentage) -> f32 {
    match value.expand() {
        ExpandedLengthPercentage::Length(points) => points,
        ExpandedLengthPercentage::Percent(_) => 0.0,
    }
}

fn auto_points(value: LengthPercentageAuto) -> Option<f32> {
    match value.expand() {
        ExpandedLengthPercentageAuto::Length(points) => Some(points),
        ExpandedLengthPercentageAuto::Auto | ExpandedLengthPercentageAuto::Percent(_) => None,
    }
}

fn axis_size_of(value: Dimension) -> AxisSize {
    match value.expand() {
        ExpandedDimension::Length(points) => AxisSize::Points(points),
        ExpandedDimension::Percent(fraction) => AxisSize::Fraction(fraction),
        _ => AxisSize::Auto,
    }
}

fn grid_line_of(value: GridPlacement) -> GridLine {
    match value {
        GridPlacement::Line(line) => GridLine::Line(line.as_i16()),
        GridPlacement::Span(count) => GridLine::Span(count),
        _ => GridLine::Auto,
    }
}

fn margin_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentageAuto>, Error> {
    edges_rect(edges)
}

fn align_self(alignment: Alignment) -> AlignSelf {
    match alignment {
        Alignment::Start => AlignSelf::START,
        Alignment::Center => AlignSelf::CENTER,
        Alignment::End => AlignSelf::END,
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
        Justify::Start => JustifyContent::START,
        Justify::Center => JustifyContent::CENTER,
        Justify::End => JustifyContent::END,
        Justify::SpaceBetween => JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => JustifyContent::SPACE_EVENLY,
    }
}

/// Size already laid out for this text, when `max_width` cannot change the lines.
///
/// Taffy asks once at max-content and again at the definite width. The max-content
/// pass uses a million-point constraint, which misses the frozen glyphs and builds a
/// paragraph that is thrown away. A result that already fits inside its old
/// constraint is the same at any wider limit, including that probe.
fn reusable_text_metrics(
    text: &super::node::TextPayload,
    font_size: f32,
    max_width: f32,
) -> Option<&crate::text::ParagraphMetrics> {
    let cached = text.cached.as_ref()?;
    let metrics = text.metrics.as_ref()?;
    let key = cached;
    if key.font_size != font_size.to_bits() || key.text.as_ref() != text.utf8.as_ref() {
        return None;
    }
    if key.style.as_ref() != text.style.as_ref() {
        return None;
    }
    if !spans_match(text, cached) {
        return None;
    }
    let old = f32::from_bits(key.max_width);
    let size = metrics.size;
    if key.max_width == max_width.to_bits() {
        return Some(metrics);
    }
    if size.width + 0.5 < old && size.width <= max_width + 0.5 {
        return Some(metrics);
    }
    None
}

fn spans_match(text: &super::node::TextPayload, cached: &crate::text::MetricsKey) -> bool {
    if text.spans.len() != cached.spans.len() {
        return false;
    }
    text.spans
        .iter()
        .zip(cached.spans.iter())
        .all(|(span, metric)| {
            span.utf8 == metric.utf8.as_ref()
                && span.weight == metric.weight
                && span.underline == metric.underline
                && span.line_through == metric.line_through
                && span.family.as_deref().unwrap_or("") == metric.family.as_ref()
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
    shaper: &'a mut dyn Shaper,
    text_scale: f32,
    text_reused: &'a Cell<u32>,
    text_shaped: &'a Cell<u32>,
    /// Measurements already shaped in this Taffy pass, keyed by node index.
    provisional: HashMap<usize, (f32, Arc<crate::text::ParagraphMetrics>)>,
}

fn leaf_output(
    inputs: taffy::LayoutInput,
    style: &Style,
    measure: impl FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> (Size<f32>, Option<f32>),
) -> taffy::LayoutOutput {
    let found = Cell::new(None);
    let mut output = taffy::compute_leaf_layout(
        inputs,
        style,
        |_, _| 0.0,
        |known, available| {
            let (size, baseline) = measure(known, available);
            found.set(baseline);
            size
        },
    );
    if let Some(baseline) = found.get() {
        use taffy::util::ResolveOrZero;
        let width = inputs.parent_size.width;
        let padding = style.padding.resolve_or_zero(width, |_, _| 0.0);
        let border = style.border.resolve_or_zero(width, |_, _| 0.0);
        output.baselines.first = Some(baseline + padding.top + border.top);
    }
    output
}

fn measure_leaf(
    env: &mut MeasureEnv<'_>,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<usize>,
    measured: &mut Vec<NodeId>,
) -> (Size<f32>, Option<f32>) {
    let Some(index) = context else {
        return (Size::ZERO, None);
    };
    let Some(slot) = env.slots.get(index) else {
        return (Size::ZERO, None);
    };
    let Some(node) = slot.node.as_ref() else {
        return (Size::ZERO, None);
    };
    let mut baseline = None;
    let size = if let Some(text) = node.text() {
        // A min-content query uses the shaper's intrinsic width for the full
        // metrics key: family, weight, and runs, not a whitespace split alone.
        if matches!(available.width, AvailableSpace::MinContent) {
            let font_size = text.font_size * env.text_scale;
            let key = crate::text::metrics_key(
                Arc::clone(&text.utf8),
                Arc::clone(&text.style),
                text.spans.as_ref(),
                font_size,
                SINGLE_LINE_MAX,
            );
            let (width, metrics) = env.shaper.min_content_metrics(&key);
            baseline = Some(metrics.first_baseline());
            let size = TextSize {
                width,
                height: metrics.size.height,
            };
            env.provisional
                .entry(index)
                .or_insert((SINGLE_LINE_MAX, metrics));
            size
        } else {
            let max_width = line_limit(available.width);
            let font_size = text.font_size * env.text_scale;
            let width_bits = max_width.to_bits();
            if let Some(metrics) = reusable_text_metrics(text, font_size, max_width) {
                env.text_reused.set(env.text_reused.get().saturating_add(1));
                let cached = text.cached.as_ref();
                let paints_at = cached.map(|key| key.max_width).unwrap_or(0);
                if paints_at == width_bits {
                    text.max_width.set(max_width);
                }
                if let Some(cached) = cached {
                    env.shaper.touch(cached);
                }
                baseline = Some(metrics.first_baseline());
                metrics.size
            } else if let Some((_, metrics)) =
                env.provisional.get(&index).filter(|(width, metrics)| {
                    *width == max_width
                        || (metrics.size.width + 0.5 < *width
                            && metrics.size.width <= max_width + 0.5)
                })
            {
                env.text_reused.set(env.text_reused.get().saturating_add(1));
                if max_width != SINGLE_LINE_MAX && text.max_width.replace(max_width) != max_width {
                    measured.push(NodeId {
                        index: crate::cast::u32_from_usize(index),
                        generation: slot.generation,
                    });
                }
                baseline = Some(metrics.first_baseline());
                metrics.size
            } else {
                let previous = text.max_width.get();
                text.max_width.set(max_width);
                if previous != max_width {
                    measured.push(NodeId {
                        index: crate::cast::u32_from_usize(index),
                        generation: slot.generation,
                    });
                }
                let key = crate::text::metrics_key(
                    Arc::clone(&text.utf8),
                    Arc::clone(&text.style),
                    text.spans.as_ref(),
                    font_size,
                    max_width,
                );
                env.text_shaped.set(env.text_shaped.get().saturating_add(1));
                let metrics = env.shaper.metrics(&key);
                baseline = Some(metrics.first_baseline());
                let size = metrics.size;
                env.provisional.insert(index, (max_width, metrics));
                size
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
    (
        Size {
            width: known.width.unwrap_or(size.width),
            height: known.height.unwrap_or(size.height),
        },
        baseline,
    )
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
    text.cached = Some(crate::text::metrics_key(
        Arc::clone(&text.utf8),
        Arc::clone(&text.style),
        &[],
        16.0,
        200.0,
    ));
    text.metrics = Some(Arc::new(crate::text::ParagraphMetrics::with_baseline(
        TextSize {
            width: 48.0,
            height: 16.0,
        },
        Arc::from([]),
        12.0,
    )));
    let exact = reusable_text_metrics(&text, 16.0, 200.0).expect("same width");
    assert_eq!(exact.size.width, 48.0);
    let wider = reusable_text_metrics(&text, 16.0, SINGLE_LINE_MAX).expect("max content");
    assert_eq!(wider.size.width, 48.0);
    assert!(reusable_text_metrics(&text, 16.0, 20.0).is_none());

    text.metrics = Some(Arc::new(crate::text::ParagraphMetrics::with_baseline(
        TextSize {
            width: 200.0,
            height: 32.0,
        },
        Arc::from([]),
        12.0,
    )));
    assert!(reusable_text_metrics(&text, 16.0, SINGLE_LINE_MAX).is_none());
    assert!(reusable_text_metrics(&text, 16.0, 200.0).is_some());
}

#[test]
fn relayout_boundary_moves_siblings_and_skips_a_clean_scroll() -> Result<(), Error> {
    let mut tree = Tree::new(crate::text::MonoShaper::default());
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

#[test]
fn fixed_boxes_relayout_with_their_parent_input_and_match_a_root_pass() -> Result<(), Error> {
    fn fixture(absolute: bool) -> Result<(Tree, [NodeId; 4]), Error> {
        let mut tree = Tree::new(crate::text::MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 161.75, 90.5)?;
        let root = tree.column(0.0)?;
        tree.set_size(root, 160.5, 90.5)?;
        tree.set_padding(root, 2.25)?;
        let fixed = tree.sized(96.5, 72.5)?;
        if absolute {
            tree.set_position(fixed, true)?;
            tree.set_inset(
                fixed,
                Inset {
                    left: Some(3.25),
                    top: Some(5.5),
                    right: None,
                    bottom: None,
                },
            )?;
        }
        let moveable = tree.decorated([50, 140, 220, 255], 0.0)?;
        tree.set_size(moveable, 20.0, 24.0)?;
        tree.set_pointer_target(moveable)?;
        tree.set_position(moveable, true)?;
        let full = tree.sized(96.5, 72.5)?;
        tree.add_child(root, fixed)?;
        tree.add_child(fixed, full)?;
        tree.add_child(fixed, moveable)?;
        tree.layout()?;
        tree.record_layers()?;
        let _ = tree.take_layout_trace();
        Ok((tree, [root, fixed, full, moveable]))
    }
    for absolute in [false, true] {
        let (mut local, ids) = fixture(absolute)?;
        let (mut reference, reference_ids) = fixture(absolute)?;
        assert_eq!(ids, reference_ids);
        for tree in [&mut local, &mut reference] {
            tree.set_inset(
                ids[3],
                Inset {
                    left: Some(29.5),
                    top: Some(1.25),
                    right: None,
                    bottom: None,
                },
            )?;
        }
        let root_taffy = reference.node(ids[0])?.taffy.ok_or(Error::NoRoot)?;
        reference.taffy.dirty_root(root_taffy, "reference");
        for tree in [&mut local, &mut reference] {
            tree.layout()?;
            tree.record_layers()?;
        }
        let trace = local.take_layout_trace();
        assert_eq!(
            (trace.root_layouts, trace.local_layouts),
            (0, 1),
            "absolute={absolute}"
        );
        assert_eq!(reference.take_layout_trace().root_layouts, 1);
        for id in ids {
            assert_eq!(local.rect(id)?, reference.rect(id)?);
            let local_layout = local
                .taffy
                .layout(local.node(id)?.taffy.ok_or(Error::UnknownNode)?)?;
            let reference_layout = reference
                .taffy
                .layout(reference.node(id)?.taffy.ok_or(Error::UnknownNode)?)?;
            assert_eq!(local_layout, reference_layout);
        }
        assert_eq!(local.record()?, reference.record()?);
        assert_eq!(local.hit_root(38.0, 15.0), Some(ids[3]));
        assert_eq!(local.hit_root(38.0, 15.0), reference.hit_root(38.0, 15.0));

        for tree in [&mut local, &mut reference] {
            tree.set_size(ids[3], 233.0, 24.0)?;
        }
        reference.taffy.dirty_root(root_taffy, "reference");
        local.layout()?;
        reference.layout()?;
        assert_eq!(local.take_layout_trace().root_layouts, 1);
        assert_eq!(local.rect(ids[3])?, reference.rect(ids[3])?);
        assert_eq!(local.record()?, reference.record()?);
    }
    Ok(())
}

#[test]
fn same_output_text_reuses_layout_until_width_or_baseline_changes() -> Result<(), Error> {
    let mut tree = Tree::new(crate::text::MonoShaper::default());
    tree.trace_text_probes(true);
    let row = tree.row(0.0)?;
    tree.set_row_cross(row, super::RowCrossAlign::Baseline)?;
    let quote = tree.text("43.12", 16.0, [255; 4])?;
    let label = tree.text("last", 11.0, [255; 4])?;
    tree.add_child(row, quote)?;
    tree.add_child(row, label)?;
    tree.layout()?;
    let before = (tree.rect(quote)?, tree.rect(label)?);
    let _ = tree.take_layout_trace();

    tree.set_text(quote, "35.24")?;
    tree.layout()?;
    let trace = tree.take_layout_trace();
    assert_eq!((trace.same_text_layouts, trace.root_layouts), (1, 0));
    assert!(trace.text_probes.repeated > 0, "{trace:?}");
    assert_eq!(trace.text_probes.intrinsic, 1, "{trace:?}");
    assert_eq!(
        trace.text_shaped,
        trace.text_probes.queries - trace.text_probes.repeated - trace.text_probes.intrinsic,
        "{trace:?}",
    );
    assert_eq!((tree.rect(quote)?, tree.rect(label)?), before);
    let texts = crate::text::test_support::texts(&tree.record()?);
    assert!(texts.contains(&"35.24".to_owned()), "{texts:?}");

    tree.set_text(quote, "-123.45")?;
    tree.layout()?;
    assert_eq!(tree.take_layout_trace().root_layouts, 1);
    assert!(tree.rect(label)?.x > before.1.x);

    tree.set_font_size(quote, 28.0)?;
    tree.layout()?;
    assert_eq!(tree.take_layout_trace().root_layouts, 1);
    assert!(tree.rect(label)?.y > before.1.y);
    Ok(())
}

#[test]
fn fixed_text_keeps_layout_when_new_ink_stays_inside_its_box() -> Result<(), Error> {
    let mut tree = Tree::new(crate::text::MonoShaper::default());
    let row = tree.row(0.0)?;
    let quote = tree.text("9.99", 16.0, [255; 4])?;
    let next = tree.text("next", 16.0, [255; 4])?;
    tree.set_axis_size(quote, AxisSize::Points(80.0), AxisSize::Points(24.0))?;
    tree.set_min_size(quote, 0.0, 0.0)?;
    tree.add_child(row, quote)?;
    tree.add_child(row, next)?;
    tree.layout()?;
    let before = (tree.rect(quote)?, tree.rect(next)?);
    let _ = tree.take_layout_trace();

    tree.set_text(quote, "-1,034.56")?;
    tree.layout()?;
    let trace = tree.take_layout_trace();
    assert_eq!(
        (
            trace.root_layouts,
            trace.local_layouts,
            trace.same_text_layouts
        ),
        (0, 0, 1)
    );
    assert_eq!((tree.rect(quote)?, tree.rect(next)?), before);
    let texts = crate::text::test_support::texts(&tree.record()?);
    assert_eq!(texts, ["-1,034.56", "next"]);

    tree.set_text(quote, "9\n9\n9")?;
    tree.layout()?;
    assert_eq!(tree.take_layout_trace().root_layouts, 1);
    Ok(())
}

#[test]
fn second_layout_touches_cached_text_and_color_only_repaints() -> Result<(), Error> {
    use std::cell::Cell;
    use std::rc::Rc;

    struct Count {
        inner: crate::text::MonoShaper,
        touches: Rc<Cell<u32>>,
        metrics: Rc<Cell<u32>>,
    }

    impl Shaper for Count {
        fn metrics(&mut self, key: &crate::text::MetricsKey) -> Arc<crate::text::ParagraphMetrics> {
            self.metrics.set(self.metrics.get().saturating_add(1));
            self.inner.metrics(key)
        }
        fn touch(&mut self, _key: &crate::text::MetricsKey) {
            self.touches.set(self.touches.get().saturating_add(1));
        }
        fn caret_at_unit(
            &mut self,
            key: &crate::text::MetricsKey,
            unit: u32,
            affinity: crate::text::Affinity,
        ) -> crate::text::Caret {
            self.inner.caret_at_unit(key, unit, affinity)
        }
        fn caret_at_point(
            &mut self,
            key: &crate::text::MetricsKey,
            x: f32,
            y: f32,
        ) -> crate::text::Caret {
            self.inner.caret_at_point(key, x, y)
        }
        fn word_range(&mut self, key: &crate::text::MetricsKey, unit: u32) -> (u32, u32) {
            self.inner.word_range(key, unit)
        }
    }

    let touches = Rc::new(Cell::new(0));
    let metrics = Rc::new(Cell::new(0));
    let mut tree = Tree::new(Count {
        inner: crate::text::MonoShaper::new(),
        touches: Rc::clone(&touches),
        metrics: Rc::clone(&metrics),
    });
    tree.set_viewport(0.0, 0.0, 200.0, 400.0)?;
    let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
    tree.layout()?;
    let shaped = metrics.get();
    assert!(shaped > 0);
    tree.touch_measure(text)?;
    tree.layout()?;
    assert!(touches.get() >= 1);
    assert_eq!(metrics.get(), shaped);
    let passes = tree.layout_passes();
    tree.set_color(text, [255, 0, 0, 255])?;
    assert!(!tree.needs_layout());
    tree.layout()?;
    assert_eq!(tree.layout_passes(), passes);
    // A color change repaints without another shape.
    assert_eq!(metrics.get(), shaped);
    let list = tree.record()?;
    let Some(crate::scene::Command::Text {
        color,
        shaped,
        span_paints,
        ..
    }) = list
        .commands
        .iter()
        .find(|command| matches!(command, crate::scene::Command::Text { .. }))
    else {
        panic!("text command");
    };
    assert_eq!(*color, [255, 0, 0, 255]);
    assert!(span_paints.is_empty());
    assert!(shaped.as_ref().is_some_and(|shaped| !shaped.is_empty()));
    Ok(())
}
