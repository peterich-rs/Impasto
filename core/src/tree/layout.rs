//! Taffy styles, measure, and the viewport.

use protocol::{TextAlign, TextStyle};
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

use super::node::{Kind, Slot, TextNode};
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
        self.mark_layout();
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

    pub fn clear_paint(&mut self) {
        self.needs_paint = false;
    }

    pub fn is_text(&self, id: NodeId) -> Result<bool, Error> {
        Ok(self.node(id)?.kind == Kind::Text)
    }

    /// Replaces the text. The previous geometry stays valid for hit testing
    /// until the next `layout`.
    pub fn set_text(&mut self, id: NodeId, utf8: impl Into<String>) -> Result<(), Error> {
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            text.utf8 = utf8.into();
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    pub fn set_font_size(&mut self, id: NodeId, font_size: f32) -> Result<(), Error> {
        finite_number(font_size)?;
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            text.font_size = font_size;
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    /// Foreground for text, fill for a decorated box.
    pub fn set_color(&mut self, id: NodeId, color: [u8; 4]) -> Result<(), Error> {
        let kind = self.node(id)?.kind;
        match kind {
            Kind::Text => {
                let taffy_id = {
                    let node = self.node_mut(id)?;
                    let Some(text) = node.text.as_mut() else {
                        return Err(Error::NotText);
                    };
                    text.color = color;
                    node.taffy
                };
                self.touch_measure(taffy_id)
            }
            Kind::Decorated => {
                self.node_mut(id)?.fill = Some(color);
                self.mark_paint();
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
        node.corner_radius = radius;
        self.mark_paint();
        Ok(())
    }

    /// Layer alpha, 0–255. Geometry stays until the next layout.
    pub fn set_alpha(&mut self, id: NodeId, alpha: u8) -> Result<(), Error> {
        let node = self.node_mut(id)?;
        if node.kind != Kind::Opacity {
            return Err(Error::WrongKind);
        }
        node.alpha = Some(alpha);
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
    pub fn set_fill(&mut self, id: NodeId) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: percent(1.0),
            height: percent(1.0),
        };
        style.min_size = Size {
            width: length(0.0),
            height: length(0.0),
        };
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
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(image) = node.image.as_mut() else {
                return Err(Error::WrongKind);
            };
            image.source = source.to_owned();
            image.width = width;
            image.height = height;
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    /// Fit mode and whether a missing decode fails the frame. The source string stays `asset:name`.
    pub fn set_image_mode(
        &mut self,
        id: NodeId,
        mode: protocol::ContentMode,
        required: bool,
    ) -> Result<(), Error> {
        let node = self.node_mut(id)?;
        let Some(image) = node.image.as_mut() else {
            return Err(Error::WrongKind);
        };
        image.mode = mode;
        image.required = required;
        self.mark_paint();
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
                text.utf8 = spans.iter().map(|span| span.utf8.as_str()).collect();
            }
            text.spans = spans;
            text.links = links;
        })
    }

    pub fn set_max_lines(&mut self, id: NodeId, max_lines: Option<u32>) -> Result<(), Error> {
        if matches!(max_lines, Some(0)) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style.max_lines = max_lines)
    }

    pub fn set_ellipsis(&mut self, id: NodeId, ellipsis: Option<&str>) -> Result<(), Error> {
        let ellipsis = match ellipsis {
            Some(text) if !text.is_empty() => Some(text.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| {
            if ellipsis.is_some() {
                text.style.overflow = protocol::TextOverflow::Ellipsis;
            } else if text.style.overflow == protocol::TextOverflow::Ellipsis {
                text.style.overflow = protocol::TextOverflow::Clip;
            }
            text.style.ellipsis = ellipsis;
        })
    }

    pub fn set_overflow(
        &mut self,
        id: NodeId,
        overflow: protocol::TextOverflow,
    ) -> Result<(), Error> {
        self.edit_text(id, |text| {
            text.style.overflow = overflow;
            if overflow == protocol::TextOverflow::Ellipsis && text.style.ellipsis.is_none() {
                text.style.ellipsis = Some("…".to_owned());
            }
        })
    }

    pub fn set_soft_wrap(&mut self, id: NodeId, soft_wrap: bool) -> Result<(), Error> {
        self.edit_text(id, |text| text.style.soft_wrap = soft_wrap)
    }

    pub fn set_text_align(&mut self, id: NodeId, align: TextAlign) -> Result<(), Error> {
        self.edit_text(id, |text| text.style.align = align)
    }

    pub fn set_font_weight(&mut self, id: NodeId, weight: u16) -> Result<(), Error> {
        if !matches!(weight, 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style.weight = weight)
    }

    pub fn set_text_direction(&mut self, id: NodeId, rtl: bool) -> Result<(), Error> {
        self.edit_text(id, |text| text.style.direction_rtl = rtl)
    }

    /// `0.0` uses the font's own line height.
    pub fn set_line_height(&mut self, id: NodeId, height: f32) -> Result<(), Error> {
        let height = finite_positive(height)?;
        self.edit_text(id, |text| text.style.height = height)
    }

    /// Marks every text node dirty so a text-scale change lays out again.
    pub fn mark_text_dirty(&mut self) {
        let measures: Vec<Option<taffy::NodeId>> = self
            .slots
            .iter()
            .filter_map(|slot| {
                let node = slot.node.as_ref()?;
                node.text.as_ref()?;
                Some(node.taffy)
            })
            .collect();
        for taffy_id in measures {
            let _ = self.touch_measure(taffy_id);
        }
        self.mark_layout();
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
        let node = self.node_mut(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        node.scroll = (dx, dy);
        self.mark_paint();
        Ok(())
    }

    pub fn fling_active(&self) -> bool {
        !self.fling.is_empty()
    }

    /// Starts coasting. Speeds are scene points per millisecond. A slow release stops.
    pub fn arm_fling(&mut self, id: NodeId, vx: f32, vy: f32) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        if vx.abs() < FLING_STOP && vy.abs() < FLING_STOP {
            self.fling.remove(&id);
            return Ok(());
        }
        self.fling.insert(id, (vx, vy));
        Ok(())
    }

    pub fn clear_fling(&mut self) {
        self.fling.clear();
    }

    /// Advances coasting scrolls. Returns whether any scroll is still moving.
    ///
    /// Decay matches the former `ScrollArea` step: each frame multiplies velocity
    /// by `0.95` and subtracts `velocity * dt`. The offset stays inside the
    /// content box. Paint is marked only when an offset changes.
    pub fn tick_fling(&mut self, dt_ms: f64) -> bool {
        let dt = if dt_ms.is_finite() && dt_ms > 0.0 {
            dt_ms as f32
        } else {
            16.0
        };
        let ids: Vec<NodeId> = self.fling.keys().copied().collect();
        let mut alive = false;
        for id in ids {
            let Some((vx, vy)) = self.fling.get(&id).copied() else {
                continue;
            };
            let Ok((x, y)) = self.scroll_offset(id) else {
                self.fling.remove(&id);
                continue;
            };
            let (max_x, max_y) = self.scroll_limits(id);
            let (nx, nvx) = step_fling_axis(x, vx, dt, max_x);
            let (ny, nvy) = step_fling_axis(y, vy, dt, max_y);
            if (nx - x).abs() > f32::EPSILON || (ny - y).abs() > f32::EPSILON {
                if self.set_scroll_offset(id, nx, ny).is_ok() {
                    self.scroll_notices.push(Gesture::Scroll(id, nx, ny));
                }
            }
            if nvx == 0.0 && nvy == 0.0 {
                self.fling.remove(&id);
            } else {
                self.fling.insert(id, (nvx, nvy));
                alive = true;
            }
        }
        alive
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
        let _ = self.node(id)?;
        let lines = self.editor_lines(id);
        let mut editor = self
            .editors
            .remove(&id)
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
        self.editors.insert(id, editor);
        Ok(value)
    }

    /// The editor string, after [`Tree::text_edit`]. `None` when this node has no editor.
    pub fn editor_text(&self, id: NodeId) -> Option<String> {
        self.editors
            .get(&id)
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
        let (utf8, font_size, max_width, color, style, _, _) = self.text_paint(id)?;
        let lines = self
            .shaper
            .lines(&utf8, font_size, max_width, color, &style);
        let mut rects = Vec::new();
        for line in &lines {
            let from = start.max(line.start);
            let to = end.min(line.end);
            if from >= to {
                continue;
            }
            let left = self.shaper.caret_for(
                &utf8,
                font_size,
                max_width,
                color,
                &style,
                from,
                crate::Affinity::Downstream,
            );
            let right = self.shaper.caret_for(
                &utf8,
                font_size,
                max_width,
                color,
                &style,
                to,
                crate::Affinity::Upstream,
            );
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
        let Some(editor) = self.editors.get(&id) else {
            return Vec::new();
        };
        let Ok((utf8, font_size, max_width, color, style, _, _)) = self.text_paint(id) else {
            return Vec::new();
        };
        if editor.value().text != utf8 {
            return Vec::new();
        }
        self.shaper
            .lines(&utf8, font_size, max_width, color, &style)
    }

    fn word_at(&self, id: NodeId, text: &str, unit: u32) -> (u32, u32) {
        if let Ok((utf8, font_size, max_width, color, style, _, _)) = self.text_paint(id)
            && text == utf8
        {
            return self
                .shaper
                .word_range(&utf8, font_size, max_width, color, &style, unit);
        }
        crate::shaper::whitespace_word(text, unit)
    }

    pub fn caret_for(
        &self,
        id: NodeId,
        unit: u32,
        affinity: crate::Affinity,
    ) -> Result<crate::Caret, Error> {
        let (utf8, font_size, max_width, color, style, _, _) = self.text_paint(id)?;
        Ok(self
            .shaper
            .caret_for(&utf8, font_size, max_width, color, &style, unit, affinity))
    }

    fn caret_at_local(&self, id: NodeId, x: f32, y: f32) -> Result<crate::Caret, Error> {
        let (utf8, font_size, max_width, color, style, _, _) = self.text_paint(id)?;
        Ok(self
            .shaper
            .caret_at_point(&utf8, font_size, max_width, color, &style, x, y))
    }

    pub fn text_clipboard(&self, id: NodeId) -> Result<Option<String>, Error> {
        let _ = self.node(id)?;
        Ok(self
            .editors
            .get(&id)
            .and_then(|editor| editor.clipboard().map(str::to_owned)))
    }

    fn scroll_limits(&self, id: NodeId) -> (f32, f32) {
        let Ok(node) = self.node(id) else {
            return (0.0, 0.0);
        };
        let mut content_w: f32 = 0.0;
        let mut content_h: f32 = 0.0;
        for child in &node.children {
            if let Ok(child) = self.node(*child) {
                content_w = content_w.max(child.w);
                content_h = content_h.max(child.h);
            }
        }
        ((content_w - node.w).max(0.0), (content_h - node.h).max(0.0))
    }

    pub fn scroll_offset(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.scroll)
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
        let node = self.node_mut(id)?;
        if node.kind != Kind::Transform {
            return Err(Error::WrongKind);
        }
        node.translate_x = translate_x;
        node.translate_y = translate_y;
        node.rotate_rad = rotate_rad;
        node.scale = scale;
        node.scale_y = scale;
        self.mark_paint();
        Ok(())
    }

    /// `None` or `""` uses the family registered on the player.
    pub fn set_family(&mut self, id: NodeId, family: Option<&str>) -> Result<(), Error> {
        let family = match family {
            Some(name) if !name.is_empty() => Some(name.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| text.style.family = family)
    }

    pub(super) fn edit_text(
        &mut self,
        id: NodeId,
        edit: impl FnOnce(&mut TextNode),
    ) -> Result<(), Error> {
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            edit(text);
            node.taffy
        };
        self.touch_measure(taffy_id)
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
        self.place(root, self.origin.0, self.origin.1)?;
        self.layout_passes = self.layout_passes.saturating_add(1);
        self.needs_layout = false;
        self.needs_paint = true;
        self.laid_out = true;
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
        let (utf8, font_size, max_width, color, style, origin_x, origin_y) = self.text_paint(id)?;
        let (local_x, local_y) = self.scene_to_layout(id, x, y).ok_or(Error::BadNumber)?;
        Ok(self.shaper.caret_at_point(
            &utf8,
            font_size,
            max_width,
            color,
            &style,
            local_x - origin_x,
            local_y - origin_y,
        ))
    }

    /// Caret rectangle for a UTF-16 index in a text node.
    pub fn caret_at_unit(&self, id: NodeId, unit: u32) -> Result<crate::Caret, Error> {
        let (utf8, font_size, max_width, color, style, _, _) = self.text_paint(id)?;
        Ok(self
            .shaper
            .caret_at_unit(&utf8, font_size, max_width, color, &style, unit))
    }

    /// UTF-16 word range around `unit` in a text node.
    pub fn word_range(&self, id: NodeId, unit: u32) -> Result<(u32, u32), Error> {
        let (utf8, font_size, max_width, color, style, _, _) = self.text_paint(id)?;
        Ok(self
            .shaper
            .word_range(&utf8, font_size, max_width, color, &style, unit))
    }

    fn text_paint(
        &self,
        id: NodeId,
    ) -> Result<(String, f32, f32, [u8; 4], TextStyle, f32, f32), Error> {
        let node = self.node(id)?;
        let Some(text) = &node.text else {
            return Err(Error::NotText);
        };
        Ok((
            text.utf8.clone(),
            text.font_size * self.text_scale,
            text.max_width.get(),
            text.color,
            text.style.clone(),
            node.x,
            node.y,
        ))
    }

    /// Same shaper `layout` used for this tree. `max_width` and `color` match
    /// [`Shaper::measure`], so a cursor measurement can use the paragraph the
    /// text node was laid out with.
    pub fn measure_text(
        &self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> TextSize {
        self.shaper
            .measure(utf8, font_size * self.text_scale, max_width, color, style)
    }

    pub fn rect(&self, id: NodeId) -> Result<Rect, Error> {
        let node = self.node(id)?;
        Ok(Rect {
            x: node.x,
            y: node.y,
            w: node.w,
            h: node.h,
        })
    }

    pub(super) fn write_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy;
        self.node_mut(id)?.style = style.clone();
        if let Some(taffy_id) = taffy_id {
            self.taffy.set_style(taffy_id, style)?;
        }
        self.mark_layout();
        Ok(())
    }

    pub(super) fn touch_measure(&mut self, taffy_id: Option<TaffyId>) -> Result<(), Error> {
        if let Some(taffy_id) = taffy_id {
            self.taffy.mark_dirty(taffy_id)?;
        }
        self.mark_layout();
        Ok(())
    }

    pub(super) fn place(&mut self, id: NodeId, origin_x: f32, origin_y: f32) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy.ok_or(Error::UnknownNode)?;
        let (local_x, local_y, w, h) = {
            let layout = self.taffy.layout(taffy_id)?;
            (
                layout.location.x,
                layout.location.y,
                layout.size.width,
                layout.size.height,
            )
        };
        let x = origin_x + local_x;
        let y = origin_y + local_y;
        let node = self.node_mut(id)?;
        node.x = x;
        node.y = y;
        node.w = w;
        node.h = h;
        let children = node.children.clone();
        for child in children {
            self.place(child, x, y)?;
        }
        Ok(())
    }
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

const FLING_DECAY: f32 = 0.95;
const FLING_STOP: f32 = 0.05;

fn step_fling_axis(offset: f32, velocity: f32, dt: f32, max: f32) -> (f32, f32) {
    let velocity = velocity * FLING_DECAY;
    let limit = max.max(0.0);
    let next = (offset - velocity * dt).clamp(0.0, limit);
    let stopped = velocity.abs() < FLING_STOP || next <= 0.0 || next >= limit;
    (next, if stopped { 0.0 } else { velocity })
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
        align_items: Some(AlignItems::Start),
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
    let size = if let Some(text) = &node.text {
        let max_width = line_limit(available.width);
        text.max_width.set(max_width);
        shaper.measure_spans(
            &text.utf8,
            &text.spans,
            text.font_size * text_scale,
            max_width,
            text.color,
            &text.style,
        )
    } else if let Some(image) = &node.image {
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
