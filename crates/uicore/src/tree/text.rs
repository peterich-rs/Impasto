//! Text node setters, caret, and paragraph keys.

use std::sync::Arc;

use protocol::{TextAlign, TextStyle};

use super::layout::{finite_number, finite_positive, live_node};
use super::node::{Kind, NodeFlags, TextPayload};
use super::{NodeId, Rect, Tree};
use crate::{Error, TextSize};

impl Tree {
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
            // The editor borrow has to end before `utf8` is replaced.
            let pending = if let Some(editor) = text.editor.as_mut() {
                let changed = editor.replace_keeping_composition(next);
                Ok((changed, editor.value().text.clone()))
            } else {
                Err(next)
            };
            match pending {
                Ok((changed, stored)) => {
                    text.utf8 = Arc::from(stored);
                    changed
                }
                Err(next) => {
                    let changed = text.utf8.as_ref() != next;
                    text.utf8 = Arc::from(next);
                    changed
                }
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
                    index: crate::cast::u32_from_usize(index),
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

    pub fn text_scale(&self) -> f32 {
        self.text_scale
    }

    /// Device pixels per scene point. Scroll offsets snap to whole device
    /// pixels when a motion settles, so text and hairlines land on texel
    /// boundaries. The default of 1.0 keeps scene-point rounding.
    pub fn set_pixel_scale(&mut self, scale: f32) -> Result<(), Error> {
        finite_number(scale)?;
        if scale <= 0.0 {
            return Err(Error::BadNumber);
        }
        self.pixel_scale = scale;
        Ok(())
    }

    pub fn pixel_scale(&self) -> f32 {
        self.pixel_scale
    }

    /// `value` in scene points rounded to the nearest device pixel.
    pub(crate) fn settle_pixels(&self, value: f32) -> f32 {
        (value * self.pixel_scale).round() / self.pixel_scale
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

    pub(super) fn editor_lines(&self, id: NodeId) -> Vec<crate::TextLine> {
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

    pub(super) fn word_at(&self, id: NodeId, text: &str, unit: u32) -> (u32, u32) {
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

    pub(super) fn caret_at_local(&self, id: NodeId, x: f32, y: f32) -> Result<crate::Caret, Error> {
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

    pub(super) fn text_key(&self, id: NodeId) -> Result<crate::PaintKey, Error> {
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
        let started = std::time::Instant::now();
        let ids = std::mem::take(&mut self.pending_text);
        for id in ids {
            let flagged = self.node_mut(id).ok().is_some_and(|node| {
                let flagged = node.flags.contains(NodeFlags::TEXT_DIRTY);
                node.flags.remove(NodeFlags::TEXT_DIRTY);
                flagged && node.text().is_some()
            });
            if !flagged {
                continue;
            }
            self.sync_visited = self.sync_visited.saturating_add(1);
            if self.sync_paragraph(id) {
                self.sync_synced = self.sync_synced.saturating_add(1);
            }
        }
        self.sync_ms += started.elapsed().as_secs_f64() * 1000.0;
    }

    pub(super) fn sync_paragraph(&mut self, id: NodeId) -> bool {
        let Ok(key) = self.text_key(id) else {
            return false;
        };
        if key.metrics.text.is_empty() {
            self.release_paragraph(id);
            return false;
        }
        let same = self
            .node(id)
            .ok()
            .and_then(|node| node.text())
            .and_then(|text| text.cached.as_ref())
            .is_some_and(|cached| cached == &key);
        if same {
            return false;
        }
        let old = self
            .node_mut(id)
            .ok()
            .and_then(|node| node.text_mut().and_then(|text| text.paragraph.take()));
        if let Some(old) = old {
            self.shaper.release_paint(old);
        }
        let (paragraph, metrics) = self.shaper.retain_measured(&key);
        if let Ok(node) = self.node_mut(id)
            && let Some(text) = node.text_mut()
        {
            text.metrics = Some(metrics);
            text.paragraph = Some(paragraph);
            text.cached = Some(key);
            return true;
        }
        self.shaper.release_paint(paragraph);
        false
    }

    pub(super) fn release_paragraph(&mut self, id: NodeId) {
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

    pub(super) fn note_text(&mut self, id: NodeId) {
        let Ok(node) = self.node_mut(id) else {
            return;
        };
        if node.text().is_none() || node.flags.contains(NodeFlags::TEXT_DIRTY) {
            return;
        }
        node.flags.insert(NodeFlags::TEXT_DIRTY);
        self.pending_text.push(id);
    }

    pub(super) fn note_text_tree(&mut self, id: NodeId) {
        self.note_text(id);
        let mut child = self.get(id).and_then(|node| node.first_child);
        let mut kids = Vec::new();
        while let Some(current) = child {
            kids.push(current);
            child = self.get(current).and_then(|node| node.next);
        }
        for child in kids {
            self.note_text_tree(child);
        }
    }

    /// Zeros the sync counters for this frame. `layout` and `record` then add to them.
    pub fn prepare_sync_stats(&mut self) {
        self.sync_visited = 0;
        self.sync_synced = 0;
        self.sync_ms = 0.0;
        self.taffy_ms = 0.0;
    }

    pub fn sync_stats(&self) -> (u32, u32) {
        (self.sync_visited, self.sync_synced)
    }
}
