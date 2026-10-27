//! Taffy styles, measure, and the viewport.

use std::sync::Arc;

use protocol::{TextAlign, TextStyle};
use taffy::prelude::*;

use super::node::{Kind, NodeFlags, Slot, TextPayload};
use super::{
    Alignment, AxisSize, Edges, Gesture, GridLine, Inset, Justify, NodeId, Rect, TrackSize, Tree,
};
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

    pub(super) fn mark_layout(&mut self) {
        self.needs_layout = true;
        self.needs_paint = true;
    }

    pub(super) fn mark_paint(&mut self) {
        self.needs_paint = true;
    }

    /// Diries the picture that owns `id`. A new layer also dirties its parent picture.
    pub(super) fn mark_repaint(&mut self, id: NodeId) {
        self.dirty_picture(id);
        if self.is_layer_node(id)
            && let Some(parent) = self.get(id).and_then(|node| node.parent)
        {
            self.dirty_picture(parent);
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
    }

    fn dirty_picture(&mut self, mut id: NodeId) {
        loop {
            let Ok(node) = self.node(id) else {
                return;
            };
            let parent = node.parent;
            let stop = parent.is_none() || node.is_layer_kind();
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

    pub(super) fn is_layer_node(&self, id: NodeId) -> bool {
        self.get(id).is_some_and(|node| node.is_layer_kind())
    }

    fn dirty_every_picture(&mut self) {
        let ids: Vec<NodeId> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let node = slot.node.as_ref()?;
                if node.parent.is_some() && !node.is_layer_kind() {
                    return None;
                }
                Some(NodeId {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for id in ids {
            if let Ok(node) = self.node_mut(id) {
                node.flags.insert(NodeFlags::REPAINT_DIRTY);
            }
        }
    }

    pub fn clear_paint(&mut self) {
        self.needs_paint = false;
    }

    pub fn is_text(&self, id: NodeId) -> Result<bool, Error> {
        Ok(self.node(id)?.kind == Kind::Text)
    }

    /// Replaces the text. The previous geometry stays valid for hit testing
    /// until the next `layout`.
    pub fn set_text(&mut self, id: NodeId, utf8: impl Into<String>) -> Result<(), Error> {
        let next = utf8.into();
        let changed = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text_mut() else {
                return Err(Error::NotText);
            };
            if text.editor.is_some() {
                let (changed, stored) = {
                    let editor = text.editor.as_mut().expect("editor");
                    let changed = editor.replace_keeping_composition(next);
                    (changed, editor.value().text.clone())
                };
                text.utf8 = Arc::from(stored);
                changed
            } else {
                let changed = text.utf8.as_ref() != next;
                text.utf8 = Arc::from(next);
                changed
            }
        };
        if !changed {
            return Ok(());
        }
        self.mark_semantics_chain(id);
        self.touch_measure(id)
    }

    pub fn set_font_size(&mut self, id: NodeId, font_size: f32) -> Result<(), Error> {
        finite_number(font_size)?;
        {
            let node = self.node_mut(id)?;
            let Some(text) = node.text_mut() else {
                return Err(Error::NotText);
            };
            if text.font_size == font_size {
                return Ok(());
            }
            text.font_size = font_size;
        }
        self.touch_measure(id)
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
        if !(grow >= 0.0) {
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
        let node = self.node_mut(id)?;
        if node.kind != Kind::Opacity {
            return Err(Error::WrongKind);
        }
        node.write_paint(|paint| paint.alpha = Some(alpha));
        let _ = node;
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
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
            width: percent(1.0),
            height: percent(1.0),
        };
        style.min_size = Size {
            width: length(0.0),
            height: length(0.0),
        };
        let parent_is_flex = self
            .node(id)?
            .parent
            .and_then(|parent| self.node(parent).ok())
            .is_some_and(|parent| matches!(parent.kind, Kind::Column | Kind::Row));
        if parent_is_flex {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = length(0.0);
        }
        if matches!(
            kind,
            Kind::Stack | Kind::Opacity | Kind::Transform | Kind::Clip
        ) {
            style.grid_template_columns = vec![fr(1.0)];
            style.grid_template_rows = vec![fr(1.0)];
        }
        self.write_style(id, style)
    }

    pub fn set_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Sized {
            return Err(Error::WrongKind);
        }
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: length(width),
            height: length(height),
        };
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

    /// Styled runs and UTF-16 links. Non-empty spans replace `utf8` with their concatenation.
    pub fn set_text_runs(
        &mut self,
        id: NodeId,
        spans: Vec<protocol::TextSpan>,
        links: Vec<protocol::TextLink>,
    ) -> Result<(), Error> {
        self.edit_text(id, |text| {
            if !spans.is_empty() {
                let joined: String = spans.iter().map(|span| span.utf8.as_str()).collect();
                text.utf8 = Arc::from(joined);
            }
            text.spans = Arc::from(spans);
            text.links = Arc::from(links);
        })
    }

    pub fn set_max_lines(&mut self, id: NodeId, max_lines: Option<u32>) -> Result<(), Error> {
        if matches!(max_lines, Some(0)) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style_mut().max_lines = max_lines)
    }

    pub fn set_ellipsis(&mut self, id: NodeId, ellipsis: Option<&str>) -> Result<(), Error> {
        let ellipsis = match ellipsis {
            Some(text) if !text.is_empty() => Some(text.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| {
            if ellipsis.is_some() {
                text.style_mut().overflow = protocol::TextOverflow::Ellipsis;
            } else if text.style_mut().overflow == protocol::TextOverflow::Ellipsis {
                text.style_mut().overflow = protocol::TextOverflow::Clip;
            }
            text.style_mut().ellipsis = ellipsis;
        })
    }

    pub fn set_overflow(
        &mut self,
        id: NodeId,
        overflow: protocol::TextOverflow,
    ) -> Result<(), Error> {
        self.edit_text(id, |text| {
            text.style_mut().overflow = overflow;
            if overflow == protocol::TextOverflow::Ellipsis && text.style_mut().ellipsis.is_none() {
                text.style_mut().ellipsis = Some("…".to_owned());
            }
        })
    }

    pub fn set_soft_wrap(&mut self, id: NodeId, soft_wrap: bool) -> Result<(), Error> {
        self.edit_text(id, |text| text.style_mut().soft_wrap = soft_wrap)
    }

    pub fn set_text_align(&mut self, id: NodeId, align: TextAlign) -> Result<(), Error> {
        self.edit_text(id, |text| text.style_mut().align = align)
    }

    pub fn set_font_weight(&mut self, id: NodeId, weight: u16) -> Result<(), Error> {
        if !matches!(weight, 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style_mut().weight = weight)
    }

    pub fn set_text_direction(&mut self, id: NodeId, rtl: bool) -> Result<(), Error> {
        self.edit_text(id, |text| text.style_mut().direction_rtl = rtl)
    }

    /// `0.0` uses the font's own line height.
    pub fn set_line_height(&mut self, id: NodeId, height: f32) -> Result<(), Error> {
        let height = finite_positive(height)?;
        self.edit_text(id, |text| text.style_mut().height = height)
    }

    /// Marks every text node dirty so a text-scale change lays out again.
    pub fn mark_text_dirty(&mut self) {
        let ids: Vec<NodeId> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let node = slot.node.as_ref()?;
                node.text()?;
                Some(NodeId {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for id in ids {
            let _ = self.touch_measure(id);
        }
    }

    /// Logical font sizes are multiplied by `scale` during measure and record.
    pub fn set_text_scale(&mut self, scale: f32) -> Result<(), Error> {
        finite_number(scale)?;
        if scale <= 0.0 {
            return Err(Error::BadNumber);
        }
        if (self.text_scale - scale).abs() <= f32::EPSILON {
            return Ok(());
        }
        self.text_scale = scale;
        self.mark_text_dirty();
        Ok(())
    }

    pub fn overscroll(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.overscroll())
    }

    pub fn text_scale(&self) -> f32 {
        self.text_scale
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
        let (max_x, max_y) = self.scroll_limits(id);
        let (ox, oy) = rubber_band(dx, dy, max_x, max_y);
        let applying = self.applying_motion;
        {
            let node = self.node_mut(id)?;
            let Some(geom) = node.scroll_mut() else {
                return Err(Error::WrongKind);
            };
            geom.offset_x = ox;
            geom.offset_y = oy;
            if !applying {
                geom.overscroll_x = crate::motion::OVERSCROLL_FACTOR * (dx - ox);
                geom.overscroll_y = crate::motion::OVERSCROLL_FACTOR * (dy - oy);
            }
        }
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
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
        self.motions.retain(|motion| {
            motion.node != id
                || !matches!(
                    motion.prop,
                    crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
                )
        });
        let (x, y) = self.scroll_offset(id)?;
        if vx.abs() >= 0.02 {
            self.animate(
                id,
                crate::AnimProp::ScrollX,
                Box::new(crate::Friction {
                    x0: x,
                    v0: -vx,
                    tau_ms: crate::SCROLL_TAU_MS,
                }),
            );
        }
        if vy.abs() >= 0.02 {
            self.animate(
                id,
                crate::AnimProp::ScrollY,
                Box::new(crate::Friction {
                    x0: y,
                    v0: -vy,
                    tau_ms: crate::SCROLL_TAU_MS,
                }),
            );
        }
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

    fn reveal(&mut self, id: NodeId, scroll: NodeId, edge: super::ScrollEdge) -> Result<(), Error> {
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
        let nx = nx.clamp(0.0, max_x.max(0.0));
        let ny = ny.clamp(0.0, max_y.max(0.0));
        self.spring_scroll(scroll, crate::AnimProp::ScrollX, ox, nx);
        self.spring_scroll(scroll, crate::AnimProp::ScrollY, oy, ny);
        Ok(())
    }

    fn spring_scroll(&mut self, id: NodeId, prop: crate::AnimProp, from: f32, to: f32) {
        if (from - to).abs() < f32::EPSILON {
            return;
        }
        self.animate(
            id,
            prop,
            Box::new(crate::Spring {
                x0: from,
                v0: 0.0,
                target: to,
                stiffness: crate::motion::OVERSCROLL_STIFFNESS,
                damping: crate::motion::OVERSCROLL_DAMPING,
            }),
        );
    }

    pub fn clear_fling(&mut self) {
        self.motions.retain(|motion| {
            !matches!(
                motion.prop,
                crate::AnimProp::ScrollX | crate::AnimProp::ScrollY
            )
        });
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
            let node = self.motions[index].node;
            let prop = self.motions[index].prop;
            let stop = self.apply_motion(node, prop, value) || finished;
            if stop {
                self.motion_ends.push((node, prop));
                self.motions.remove(index);
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
        });
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
        self.kick_scroll(id, crate::AnimProp::ScrollX, dx);
        self.kick_scroll(id, crate::AnimProp::ScrollY, dy);
    }

    fn kick_scroll(&mut self, id: NodeId, prop: crate::AnimProp, delta: f32) {
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
        self.motions[index].simulation = Box::new(crate::Friction {
            x0,
            v0: velocity,
            tau_ms: crate::SCROLL_TAU_MS,
        });
    }

    fn rebase_scroll(&mut self, id: NodeId, x: f32, y: f32) {
        let clock = self.clock_ms;
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
                tau_ms: crate::SCROLL_TAU_MS,
            });
        }
    }

    fn apply_motion(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
        self.applying_motion = true;
        let stop = match prop {
            crate::AnimProp::ScrollX | crate::AnimProp::ScrollY => {
                self.apply_scroll(id, prop, value)
            }
            crate::AnimProp::Opacity => {
                let alpha = value.round().clamp(0.0, 255.0) as u8;
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

    fn apply_scroll(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
        let Ok((x, y)) = self.scroll_offset(id) else {
            return true;
        };
        let (max_x, max_y) = self.scroll_limits(id);
        let (nx, ny, hit) = if prop == crate::AnimProp::ScrollX {
            let limit = max_x.max(0.0);
            let next = value.clamp(0.0, limit);
            (next, y, next <= 0.0 || next >= limit)
        } else {
            let limit = max_y.max(0.0);
            let next = value.clamp(0.0, limit);
            (x, next, next <= 0.0 || next >= limit)
        };
        if (nx - x).abs() > f32::EPSILON || (ny - y).abs() > f32::EPSILON {
            if self.set_scroll_offset(id, nx, ny).is_ok() {
                self.scroll_notices.push(Gesture::Scroll(id, nx, ny));
            }
        }
        hit
    }

    fn apply_transform_prop(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
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

    fn apply_overscroll(&mut self, id: NodeId, prop: crate::AnimProp, value: f32) -> bool {
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
        if x.abs() > f32::EPSILON {
            self.animate(
                id,
                crate::AnimProp::OverscrollX,
                Box::new(crate::Spring {
                    x0: x,
                    v0: 0.0,
                    target: 0.0,
                    stiffness: crate::motion::OVERSCROLL_STIFFNESS,
                    damping: crate::motion::OVERSCROLL_DAMPING,
                }),
            );
        }
        if y.abs() > f32::EPSILON {
            self.animate(
                id,
                crate::AnimProp::OverscrollY,
                Box::new(crate::Spring {
                    x0: y,
                    v0: 0.0,
                    target: 0.0,
                    stiffness: crate::motion::OVERSCROLL_STIFFNESS,
                    damping: crate::motion::OVERSCROLL_DAMPING,
                }),
            );
        }
    }

    pub fn take_scroll_notices(&mut self) -> Vec<Gesture> {
        std::mem::take(&mut self.scroll_notices)
    }

    /// Applies one edit to the editor stored on `id` and returns the new value.
    ///
    /// Line, word, and vertical motions use this node's paragraph when the
    /// editor string still matches the text node.
    pub fn text_edit(
        &mut self,
        id: NodeId,
        edit: crate::Edit,
    ) -> Result<crate::TextEditingValue, Error> {
        let lines = self.editor_lines(id);
        let mut editor = self
            .node_mut(id)?
            .text_mut()
            .ok_or(Error::NotText)?
            .editor
            .take()
            .unwrap_or_else(|| crate::TextEditor::new(""));
        editor.apply_in(
            edit,
            &lines,
            |text, unit| self.word_at(id, text, unit),
            |unit, affinity| {
                self.caret_for(id, unit, affinity).unwrap_or(crate::Caret {
                    unit,
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 0.0,
                    affinity,
                })
            },
            |x, y| {
                self.caret_at_local(id, x, y).unwrap_or(crate::Caret {
                    unit: 0,
                    x,
                    y,
                    w: 1.0,
                    h: 0.0,
                    affinity: crate::Affinity::Downstream,
                })
            },
        );
        let value = editor.value().clone();
        let changed = {
            let text = self.node_mut(id)?.text_mut().ok_or(Error::NotText)?;
            let changed = text.utf8.as_ref() != value.text;
            text.editor = Some(editor);
            if changed {
                text.utf8 = Arc::from(value.text.as_str());
            }
            changed
        };
        if changed {
            self.touch_measure(id)?;
        }
        Ok(value)
    }

    /// Revision and value of the editor on `id`.
    pub fn editor_snapshot(&self, id: NodeId) -> Option<(u64, crate::TextEditingValue)> {
        let editor = self.node(id).ok()?.text()?.editor.as_ref()?;
        Some((editor.revision(), editor.value().clone()))
    }

    /// The editor string, after [`Tree::text_edit`]. `None` when this node has no editor.
    pub fn editor_text(&self, id: NodeId) -> Option<String> {
        self.node(id)
            .ok()?
            .text()?
            .editor
            .as_ref()
            .map(|editor| editor.value().text.clone())
    }

    /// Rectangles covering a UTF-16 selection, in paragraph space.
    pub fn selection_rects(&self, id: NodeId, start: u32, end: u32) -> Result<Vec<Rect>, Error> {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if start == end {
            return Ok(Vec::new());
        }
        let key = self.text_key(id)?;
        let metrics = self.shaper.metrics(&key.metrics);
        let lines = &metrics.lines;
        let mut rects = Vec::new();
        for line in lines.iter() {
            let from = start.max(line.start);
            let to = end.min(line.end);
            if from >= to {
                continue;
            }
            let left = self
                .shaper
                .caret_at_unit(&key.metrics, from, crate::Affinity::Downstream);
            let right = self
                .shaper
                .caret_at_unit(&key.metrics, to, crate::Affinity::Upstream);
            let x = left.x.min(right.x);
            let w = (left.x - right.x).abs().max(1.0);
            rects.push(Rect {
                x,
                y: line.y,
                w,
                h: line.h,
            });
        }
        Ok(rects)
    }

    fn editor_lines(&self, id: NodeId) -> Vec<crate::TextLine> {
        let matches = live_node(&self.slots, id).ok().is_some_and(|node| {
            node.text().is_some_and(|text| {
                text.editor
                    .as_ref()
                    .is_some_and(|editor| editor.value().text == text.utf8.as_ref())
            })
        });
        if !matches {
            return Vec::new();
        }
        let Ok(key) = self.text_key(id) else {
            return Vec::new();
        };
        self.shaper.metrics(&key.metrics).lines.to_vec()
    }

    fn word_at(&self, id: NodeId, text: &str, unit: u32) -> (u32, u32) {
        if let Ok(key) = self.text_key(id)
            && key.metrics.text.as_ref() == text
        {
            return self.shaper.word_range(&key.metrics, unit);
        }
        crate::shaper::whitespace_word(text, unit)
    }

    pub fn caret_for(
        &self,
        id: NodeId,
        unit: u32,
        affinity: crate::Affinity,
    ) -> Result<crate::Caret, Error> {
        let key = self.text_key(id)?;
        Ok(self.shaper.caret_at_unit(&key.metrics, unit, affinity))
    }

    fn caret_at_local(&self, id: NodeId, x: f32, y: f32) -> Result<crate::Caret, Error> {
        let key = self.text_key(id)?;
        Ok(self.shaper.caret_at_point(&key.metrics, x, y))
    }

    pub fn text_clipboard(&self, id: NodeId) -> Result<Option<String>, Error> {
        let node = self.node(id)?;
        Ok(node
            .text()
            .and_then(|text| text.editor.as_ref())
            .and_then(|editor| editor.clipboard().map(str::to_owned)))
    }

    fn scroll_limits(&self, id: NodeId) -> (f32, f32) {
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
        ((content_w - width).max(0.0), (content_h - height).max(0.0))
    }

    pub fn scroll_offset(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.scroll_offset())
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
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.mark_paint();
        Ok(())
    }

    /// `None` or `""` uses the family registered on the player.
    pub fn set_family(&mut self, id: NodeId, family: Option<&str>) -> Result<(), Error> {
        let family = match family {
            Some(name) if !name.is_empty() => Some(name.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| text.style_mut().family = family)
    }

    pub(super) fn edit_text(
        &mut self,
        id: NodeId,
        edit: impl FnOnce(&mut TextPayload),
    ) -> Result<(), Error> {
        let same = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text_mut() else {
                return Err(Error::NotText);
            };
            let style = Arc::clone(&text.style);
            let color = text.color;
            let utf8 = Arc::clone(&text.utf8);
            edit(text);
            color == text.color && utf8 == text.utf8 && *style == *text.style
        };
        if same {
            return Ok(());
        }
        self.touch_measure(id)
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
        let slots = &self.slots;
        let shaper = self.shaper.as_ref();
        let text_scale = self.text_scale;
        self.taffy.compute_layout_with_measure(
            root_taffy,
            available,
            |known, available, _id, context, _style| {
                measure_leaf(slots, shaper, text_scale, known, available, context)
            },
        )?;
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
        self.needs_layout = false;
        self.needs_paint = true;
        self.laid_out = true;
        self.sync_paragraphs();
        Ok(())
    }

    /// The pointer region wins a drag on the claimed axis before scrolling does.
    pub fn set_drag_claim(&mut self, id: NodeId, claim: super::DragClaim) -> Result<(), Error> {
        self.node_mut(id)?.drag_claim = claim;
        Ok(())
    }

    /// Caret for a scene point over a text node. The returned `x` and `y` are
    /// relative to the paragraph origin.
    pub fn caret_at(&self, id: NodeId, x: f32, y: f32) -> Result<crate::Caret, Error> {
        let (local_x, local_y) = self.scene_to_layout(id, x, y).ok_or(Error::BadNumber)?;
        let (abs_x, abs_y) = self.layout_point(id)?;
        let key = self.text_key(id)?;
        Ok(self
            .shaper
            .caret_at_point(&key.metrics, local_x - abs_x, local_y - abs_y))
    }

    /// Caret rectangle for a UTF-16 index in a text node.
    pub fn caret_at_unit(&self, id: NodeId, unit: u32) -> Result<crate::Caret, Error> {
        self.caret_for(id, unit, crate::Affinity::Downstream)
    }

    /// UTF-16 word range around `unit` in a text node.
    pub fn word_range(&self, id: NodeId, unit: u32) -> Result<(u32, u32), Error> {
        let key = self.text_key(id)?;
        Ok(self.shaper.word_range(&key.metrics, unit))
    }

    /// Same shaper `layout` used for this tree.
    pub fn measure_text(
        &self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> TextSize {
        let key = crate::paint_key(
            Arc::from(utf8),
            Arc::new(style.clone()),
            &[],
            font_size * self.text_scale,
            max_width,
            color,
        );
        self.shaper.metrics(&key.metrics).size
    }

    fn text_key(&self, id: NodeId) -> Result<crate::PaintKey, Error> {
        let node = live_node(&self.slots, id)?;
        let text = node.text().ok_or(Error::NotText)?;
        Ok(crate::paint_key(
            Arc::clone(&text.utf8),
            Arc::clone(&text.style),
            text.spans.as_ref(),
            text.font_size * self.text_scale,
            text.max_width.get(),
            text.color,
        ))
    }

    pub(super) fn sync_paragraphs(&mut self) {
        let mut ids = Vec::new();
        if let Some(root) = self.root {
            self.collect_text(root, &mut ids);
        }
        for id in ids {
            self.sync_paragraph(id);
        }
    }

    fn collect_text(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.get(id) else {
            return;
        };
        if node.text().is_some() {
            out.push(id);
        }
        let mut child = node.first_child;
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            self.collect_text(current, out);
            child = next;
        }
    }

    fn sync_paragraph(&mut self, id: NodeId) {
        let Ok(key) = self.text_key(id) else {
            return;
        };
        if key.metrics.text.is_empty() {
            self.release_paragraph(id);
            return;
        }
        let same = self
            .node(id)
            .ok()
            .and_then(|node| node.text())
            .and_then(|text| text.cached.as_ref())
            .is_some_and(|cached| cached == &key);
        if same {
            return;
        }
        let old = self
            .node_mut(id)
            .ok()
            .and_then(|node| node.text_mut().and_then(|text| text.paragraph.take()));
        if let Some(old) = old {
            self.shaper.release_paint(old);
        }
        let metrics = self.shaper.metrics(&key.metrics);
        let paragraph = self.shaper.retain_paint(&key);
        if let Ok(node) = self.node_mut(id)
            && let Some(text) = node.text_mut()
        {
            text.metrics = Some(metrics);
            text.paragraph = Some(paragraph);
            text.cached = Some(key);
        }
    }

    fn release_paragraph(&mut self, id: NodeId) {
        let old = self.node_mut(id).ok().and_then(|node| {
            node.text_mut().and_then(|text| {
                text.metrics = None;
                text.cached = None;
                text.paragraph.take()
            })
        });
        if let Some(old) = old {
            self.shaper.release_paint(old);
        }
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
        self.mark_layout_from(id);
        Ok(())
    }

    pub(super) fn touch_measure(&mut self, id: NodeId) -> Result<(), Error> {
        if let Some(taffy_id) = self.node(id)?.taffy {
            self.taffy.mark_dirty(taffy_id)?;
        }
        self.mark_layout_from(id);
        Ok(())
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

    fn mark_boundary(&mut self, mut id: NodeId) {
        loop {
            let Ok(node) = self.node(id) else {
                return;
            };
            let parent = node.parent;
            let boundary = node.flags.contains(NodeFlags::RELAYOUT_BOUNDARY);
            if boundary || parent.is_none() {
                if let Ok(node) = self.node_mut(id) {
                    node.flags.insert(NodeFlags::BOUNDARY_DIRTY);
                    if parent.is_none() {
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

    fn place_boundary(&mut self, id: NodeId) -> Result<(), Error> {
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

    fn write_box_from_taffy(&mut self, id: NodeId) -> Result<(), Error> {
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

    fn shift_baseline(&mut self, row: NodeId) -> Result<(), Error> {
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

    fn node_baseline(&self, id: NodeId) -> f32 {
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

    fn clear_layout_flags(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags
                .remove(NodeFlags::LAYOUT_DIRTY | NodeFlags::BOUNDARY_DIRTY);
        }
        self.dirty_boundaries.retain(|boundary| *boundary != id);
    }
}

fn rubber_band(x: f32, y: f32, max_x: f32, max_y: f32) -> (f32, f32) {
    (x.clamp(0.0, max_x.max(0.0)), y.clamp(0.0, max_y.max(0.0)))
}

fn nearest_offset(current: f32, start: f32, end: f32, view: f32) -> f32 {
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

fn live_node(slots: &[Slot], id: NodeId) -> Result<&super::node::Node, Error> {
    let slot = slots.get(id.slot()).ok_or(Error::UnknownNode)?;
    if slot.generation != id.generation {
        return Err(Error::UnknownNode);
    }
    slot.node.as_ref().ok_or(Error::UnknownNode)
}

pub(super) fn finite_number(value: f32) -> Result<(), Error> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(Error::BadNumber)
    }
}

fn finite_positive(value: f32) -> Result<f32, Error> {
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

fn edge_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentage>, Error> {
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

fn margin_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentageAuto>, Error> {
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
            width: length(0.0),
            height: length(0.0),
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

fn line_limit(space: AvailableSpace) -> f32 {
    match space {
        AvailableSpace::Definite(width) if width > 0.0 => width,
        AvailableSpace::Definite(_) => 0.0,
        AvailableSpace::MinContent | AvailableSpace::MaxContent => SINGLE_LINE_MAX,
    }
}

fn measure_leaf(
    slots: &[Slot],
    shaper: &dyn Shaper,
    text_scale: f32,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut usize>,
) -> Size<f32> {
    let Some(index) = context.copied() else {
        return Size::ZERO;
    };
    let Some(slot) = slots.get(index) else {
        return Size::ZERO;
    };
    let Some(node) = slot.node.as_ref() else {
        return Size::ZERO;
    };
    let size = if let Some(text) = node.text() {
        let max_width = line_limit(available.width);
        text.max_width.set(max_width);
        let key = crate::paint_key(
            Arc::clone(&text.utf8),
            Arc::clone(&text.style),
            text.spans.as_ref(),
            text.font_size * text_scale,
            max_width,
            text.color,
        );
        shaper.metrics(&key.metrics).size
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
