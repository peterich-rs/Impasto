use std::ffi::CString;
use std::ptr;

use super::Typography;
use super::key::{ParaKey, Shaped, hash_key, hash_query, key_matches, para_key};
use super::paint_store::PaintSlot;
use crate::player::ffi;
use crate::player::{
    apply_decoration, impeller_align, impeller_weight, paint_color, push_runs, read_lines,
    utf16_len,
};
use crate::{CaretHit, LineMetric, Status};
use protocol::{TextSpan, TextStyle};

impl Typography {
    /// Shapes `utf8` with `color` on this engine's typography context.
    ///
    /// The paragraph is kept and drawn by the next [`Engine::submit_layers`] that
    /// carries the same text, size, width, and color.
    pub fn measure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<(f32, f32), Status> {
        self.measure_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    /// Same paragraph as [`Engine::measure_paragraph`], including each span in the cache key.
    pub fn measure_with_spans(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<(f32, f32), Status> {
        let owned;
        let utf8 = if spans.is_empty() {
            utf8
        } else {
            owned = spans
                .iter()
                .map(|span| span.utf8.as_str())
                .collect::<String>();
            debug_assert_eq!(owned, utf8);
            &owned
        };
        if utf8.is_empty() {
            return Err(Status::BadScene);
        }
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let font_bits = font_size.to_bits();
        let width_bits = max_width.to_bits();
        let ellipsis = style.ellipsis.as_deref().unwrap_or("");
        let hash = hash_query(
            utf8,
            font_bits,
            width_bits,
            color,
            &family,
            style.max_lines.unwrap_or(0),
            ellipsis,
            style.align,
            style.weight,
            style.direction_rtl,
            style.height.to_bits(),
            style.underline,
            style.line_through,
            style.soft_wrap,
            style.overflow,
            spans,
        );
        if !self.stale
            && let Some(found) = self.measured.get_mut(hash, |key| {
                key_matches(
                    key,
                    utf8,
                    font_bits,
                    width_bits,
                    color,
                    &family,
                    style.max_lines.unwrap_or(0),
                    ellipsis,
                    style.align,
                    style.weight,
                    style.direction_rtl,
                    style.height.to_bits(),
                    style.underline,
                    style.line_through,
                    style.soft_wrap,
                    style.overflow,
                    spans,
                )
            })
        {
            found.used = self.generation;
            let size = (found.width, found.height);
            self.stats.measure_hit = self.stats.measure_hit.saturating_add(1);
            return Ok(size);
        }
        self.stats.measure_miss = self.stats.measure_miss.saturating_add(1);
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, style, spans) else {
            return Err(Status::BadScene);
        };
        let mut shaped = self.build_paragraph(utf8, font_size, max_width, color, style, spans)?;
        self.stats.built = self.stats.built.saturating_add(1);
        shaped.used = self.generation;
        if style.overflow != protocol::TextOverflow::Visible {
            shaped.width = shaped.width.min(max_width);
        }
        let size = (shaped.width, shaped.height);
        self.store_paragraph(key, shaped);
        Ok(size)
    }

    /// Caret nearest a point in paragraph space. The paragraph is the one
    /// [`Engine::measure_paragraph`] caches.
    pub fn caret_at_point(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Result<crate::CaretHit, Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        // SAFETY: `paragraph` is a live cached paragraph owned by this engine.
        // The glyph info is released before return. Bounds are written by Impeller.
        unsafe {
            let info = ffi::ImpellerParagraphCreateGlyphInfoAtParagraphCoordinatesNew(
                paragraph,
                f64::from(x),
                f64::from(y),
            );
            if info.is_null() {
                return Ok(crate::CaretHit {
                    unit: utf16_len(utf8),
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: font_size.max(0.0),
                    upstream: false,
                });
            }
            let begin = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeBegin(info);
            let end = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeEnd(info);
            let mut bounds = ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            };
            ffi::ImpellerGlyphInfoGetGraphemeClusterBounds(info, &mut bounds);
            ffi::ImpellerGlyphInfoRelease(info);
            let mid = bounds.x + bounds.width * 0.5;
            let at_end = x >= mid;
            Ok(crate::CaretHit {
                unit: u32::try_from(if at_end { end } else { begin }).unwrap_or(0),
                x: if at_end {
                    bounds.x + bounds.width
                } else {
                    bounds.x
                },
                y: bounds.y,
                w: 1.0,
                h: bounds.height,
                upstream: false,
            })
        }
    }

    /// Caret rectangle for a UTF-16 index in the cached paragraph.
    pub fn caret_at_unit(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> Result<crate::CaretHit, Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        let end_unit = utf16_len(utf8);
        let unit = unit.min(end_unit);
        if unit == end_unit {
            let lines = self.lines(utf8, font_size, max_width, color, style)?;
            if let Some(last) = lines.last() {
                return Ok(crate::CaretHit {
                    unit,
                    x: last.x + last.w,
                    y: last.y,
                    w: 1.0,
                    h: last.h,
                    upstream: false,
                });
            }
            return Ok(crate::CaretHit {
                unit,
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: font_size.max(0.0),
                upstream: false,
            });
        }
        // SAFETY: `paragraph` is a live cached paragraph. Glyph info is released here.
        unsafe {
            let info =
                ffi::ImpellerParagraphCreateGlyphInfoAtCodeUnitIndexNew(paragraph, unit as usize);
            if info.is_null() {
                return Ok(crate::CaretHit {
                    unit,
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: font_size.max(0.0),
                    upstream: false,
                });
            }
            let begin = ffi::ImpellerGlyphInfoGetGraphemeClusterCodeUnitRangeBegin(info);
            let mut bounds = ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            };
            ffi::ImpellerGlyphInfoGetGraphemeClusterBounds(info, &mut bounds);
            ffi::ImpellerGlyphInfoRelease(info);
            let x = if unit as usize > begin {
                bounds.x + bounds.width
            } else {
                bounds.x
            };
            Ok(crate::CaretHit {
                unit,
                x,
                y: bounds.y,
                w: 1.0,
                h: bounds.height,
                upstream: false,
            })
        }
    }

    /// UTF-16 word bounds around `unit`, from the cached paragraph.
    pub fn word_range(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> Result<(u32, u32), Status> {
        let paragraph = self.ensure_paragraph(utf8, font_size, max_width, color, style)?;
        let mut range = ffi::ImpellerRange { start: 0, end: 0 };
        // SAFETY: `paragraph` is a live cached paragraph. `range` is a local out-param.
        unsafe {
            ffi::ImpellerParagraphGetWordBoundary(paragraph, unit as usize, &mut range);
        }
        Ok((
            u32::try_from(range.start).unwrap_or(0),
            u32::try_from(range.end).unwrap_or(0),
        ))
    }

    /// Laid-out lines of the cached paragraph.
    pub fn lines(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<Vec<crate::LineMetric>, Status> {
        self.lines_with_spans(utf8, font_size, max_width, color, style, &[])
    }

    /// Laid-out lines. Span runs use the same cache entry as [`Engine::measure_with_spans`].
    pub fn lines_with_spans(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<Vec<crate::LineMetric>, Status> {
        self.measure_with_spans(utf8, font_size, max_width, color, style, spans)?;
        let paragraph = self.cached_paragraph(utf8, font_size, max_width, color, style, spans)?;
        // SAFETY: `paragraph` is cached and alive. Line metrics stay with it.
        unsafe { read_lines(paragraph) }
    }

    /// Caret at `unit`. `upstream` selects the previous line at a soft break.
    pub fn caret_for(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
        upstream: bool,
    ) -> Result<crate::CaretHit, Status> {
        let lines = self.lines(utf8, font_size, max_width, color, style)?;
        if upstream
            && let Some((index, _)) = lines
                .iter()
                .enumerate()
                .find(|(_, line)| line.start == unit)
            && index > 0
            && !lines[index - 1].hard_break
        {
            let prev = &lines[index - 1];
            return Ok(crate::CaretHit {
                unit,
                x: prev.x + prev.w,
                y: prev.y,
                w: 1.0,
                h: prev.h,
                upstream: true,
            });
        }
        self.caret_at_unit(utf8, font_size, max_width, color, style, unit)
    }

    pub(crate) fn ensure_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Result<ffi::ImpellerParagraph, Status> {
        self.measure_paragraph(utf8, font_size, max_width, color, style)?;
        self.cached_paragraph(utf8, font_size, max_width, color, style, &[])
    }

    pub(crate) fn cached_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<ffi::ImpellerParagraph, Status> {
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let ellipsis = style.ellipsis.as_deref().unwrap_or("");
        let hash = hash_query(
            utf8,
            font_size.to_bits(),
            max_width.to_bits(),
            color,
            &family,
            style.max_lines.unwrap_or(0),
            ellipsis,
            style.align,
            style.weight,
            style.direction_rtl,
            style.height.to_bits(),
            style.underline,
            style.line_through,
            style.soft_wrap,
            style.overflow,
            spans,
        );
        self.measured
            .get_mut(hash, |key| {
                key_matches(
                    key,
                    utf8,
                    font_size.to_bits(),
                    max_width.to_bits(),
                    color,
                    &family,
                    style.max_lines.unwrap_or(0),
                    ellipsis,
                    style.align,
                    style.weight,
                    style.direction_rtl,
                    style.height.to_bits(),
                    style.underline,
                    style.line_through,
                    style.soft_wrap,
                    style.overflow,
                    spans,
                )
            })
            .map(|shaped| shaped.paragraph)
            .ok_or(Status::BadScene)
    }

    /// Retains a colored paragraph. The same key bumps the reference count and
    /// returns the existing id. A released slot is reused with a new generation.
    pub fn retain_paragraph(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<protocol::ParagraphId, Status> {
        self.paint_slot(utf8, font_size, max_width, color, style, spans, true)
            .map(|(id, _, _)| id)
    }

    /// Measures with the real foreground. A miss builds the paint slot at `refs == 0`.
    pub fn measure_paint(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<(f32, f32), Status> {
        self.paint_slot(utf8, font_size, max_width, color, style, spans, false)
            .map(|(_, width, height)| (width, height))
    }

    /// Retains the paint slot, including one just built by [`Engine::measure_paint`].
    pub fn retain_measured(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<(protocol::ParagraphId, f32, f32, Vec<crate::LineMetric>, f32), Status> {
        let (id, width, height) =
            self.paint_slot(utf8, font_size, max_width, color, style, spans, true)?;
        let Some(paragraph) = self.live_paint(id) else {
            return Err(Status::BadScene);
        };
        // SAFETY: `paragraph` is the slot just retained. Line metrics stay with it.
        let lines = unsafe { read_lines(paragraph)? };
        let baseline = lines.first().map(|line| line.baseline).unwrap_or(0.0);
        Ok((id, width, height, lines, baseline))
    }

    /// Paragraph build and lookup counts since the previous take.
    pub fn take_shape_stats(&mut self) -> crate::ShapeStats {
        std::mem::take(&mut self.stats)
    }

    /// `retain` increments `refs`. Measurement leaves a new slot at `refs == 0`.
    pub(crate) fn paint_slot(
        &mut self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        spans: &[TextSpan],
        retain: bool,
    ) -> Result<(protocol::ParagraphId, f32, f32), Status> {
        let owned;
        let utf8 = if spans.is_empty() {
            utf8
        } else {
            owned = spans
                .iter()
                .map(|span| span.utf8.as_str())
                .collect::<String>();
            &owned
        };
        let Some(family) = self.resolved_family(style.family.as_deref()) else {
            return Err(Status::BadScene);
        };
        let font_bits = font_size.to_bits();
        let width_bits = max_width.to_bits();
        let ellipsis = style.ellipsis.as_deref().unwrap_or("");
        let hash = hash_query(
            utf8,
            font_bits,
            width_bits,
            color,
            &family,
            style.max_lines.unwrap_or(0),
            ellipsis,
            style.align,
            style.weight,
            style.direction_rtl,
            style.height.to_bits(),
            style.underline,
            style.line_through,
            style.soft_wrap,
            style.overflow,
            spans,
        );
        let matches = |key: &ParaKey| {
            key_matches(
                key,
                utf8,
                font_bits,
                width_bits,
                color,
                &family,
                style.max_lines.unwrap_or(0),
                ellipsis,
                style.align,
                style.weight,
                style.direction_rtl,
                style.height.to_bits(),
                style.underline,
                style.line_through,
                style.soft_wrap,
                style.overflow,
                spans,
            )
        };
        if !self.stale
            && let Some(index) = self.find_paint(hash, matches)
        {
            let slot = &mut self.paints.slots[index];
            if retain {
                slot.refs = slot.refs.saturating_add(1);
            }
            slot.used = self.generation;
            let size = (slot.width, slot.height);
            let id = protocol::ParagraphId {
                index: index as u32,
                generation: slot.generation,
            };
            if retain {
                self.stats.paint_hit = self.stats.paint_hit.saturating_add(1);
            } else {
                self.stats.measure_hit = self.stats.measure_hit.saturating_add(1);
            }
            return Ok((id, size.0, size.1));
        }
        if retain {
            self.stats.paint_miss = self.stats.paint_miss.saturating_add(1);
        } else {
            self.stats.measure_miss = self.stats.measure_miss.saturating_add(1);
        }
        let Some(key) = para_key(utf8, font_size, max_width, color, &family, style, spans) else {
            return Err(Status::BadScene);
        };
        let shaped = self.build_paragraph(utf8, font_size, max_width, color, style, spans)?;
        self.stats.built = self.stats.built.saturating_add(1);
        let width = if style.overflow != protocol::TextOverflow::Visible {
            shaped.width.min(max_width)
        } else {
            shaped.width
        };
        let index = if let Some(index) = self.paints.free.pop() {
            index as usize
        } else {
            self.paints.slots.push(PaintSlot::empty());
            self.paints.slots.len() - 1
        };
        let hash = hash_key(&key);
        let slot = &mut self.paints.slots[index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.refs = u32::from(retain);
        slot.color = color;
        slot.width = width;
        slot.height = shaped.height;
        slot.used = self.generation;
        slot.paragraph = shaped.paragraph;
        slot.key = Some(key);
        self.paints
            .index
            .entry(hash)
            .or_default()
            .push(index as u32);
        Ok((
            protocol::ParagraphId {
                index: index as u32,
                generation: self.paints.slots[index].generation,
            },
            width,
            shaped.height,
        ))
    }

    pub(crate) fn find_paint(
        &self,
        hash: u64,
        matches: impl Fn(&ParaKey) -> bool,
    ) -> Option<usize> {
        let bucket = self.paints.index.get(&hash)?;
        bucket.iter().copied().find_map(|index| {
            let slot = self.paints.slots.get(index as usize)?;
            let key = slot.key.as_ref()?;
            if slot.paragraph.is_null() || !matches(key) {
                None
            } else {
                Some(index as usize)
            }
        })
    }

    pub(crate) fn unindex_paint(&mut self, hash: u64, index: u32) {
        let Some(bucket) = self.paints.index.get_mut(&hash) else {
            return;
        };
        bucket.retain(|slot| *slot != index);
        if bucket.is_empty() {
            self.paints.index.remove(&hash);
        }
    }

    /// Drops one retain. The slot pointer is cleared before it is released, so a
    /// stale id cannot observe the new color or the freed paragraph.
    ///
    /// A measure slot stays at `refs == 0` and is not released here.
    pub fn release_paragraph(&mut self, id: protocol::ParagraphId) {
        let Some(slot) = self.paints.slots.get_mut(id.index as usize) else {
            return;
        };
        if slot.generation != id.generation || slot.refs == 0 {
            return;
        }
        slot.refs -= 1;
        if slot.refs > 0 {
            return;
        }
        let paragraph = slot.paragraph;
        let key = slot.key.take();
        slot.paragraph = ptr::null_mut();
        slot.color = [0, 0, 0, 0];
        slot.width = 0.0;
        slot.height = 0.0;
        self.paints.free.push(id.index);
        if let Some(key) = key {
            let hash = hash_key(&key);
            self.unindex_paint(hash, id.index);
        }
        if !paragraph.is_null() {
            self.retire_paragraph(paragraph);
        }
    }

    /// True when `id` still names a live retained paragraph.
    pub fn paragraph_live(&self, id: protocol::ParagraphId) -> bool {
        self.paints
            .slots
            .get(id.index as usize)
            .is_some_and(|slot| {
                slot.generation == id.generation && slot.refs > 0 && !slot.paragraph.is_null()
            })
    }

    /// `(rebuilt_from_payload, color)` of the last text command submitted.
    pub fn text_paint_probe(&self) -> (bool, [u8; 4]) {
        (self.probe_rebuilt, self.probe_color)
    }

    pub(crate) fn live_paint(&self, id: protocol::ParagraphId) -> Option<ffi::ImpellerParagraph> {
        let slot = self.paints.slots.get(id.index as usize)?;
        if slot.generation == id.generation && slot.refs > 0 && !slot.paragraph.is_null() {
            Some(slot.paragraph)
        } else {
            None
        }
    }

    pub(crate) fn build_paragraph(
        &self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        text_style: &TextStyle,
        spans: &[TextSpan],
    ) -> Result<Shaped, Status> {
        // SAFETY: the paint is released before this returns. A null paint cannot shape.
        let paint = unsafe { ffi::ImpellerPaintNew() };
        if paint.is_null() {
            return Err(Status::BadScene);
        }
        paint_color(paint, color);
        let paragraph = self.shape(
            Some(paint),
            utf8,
            font_size,
            max_width,
            color,
            text_style,
            spans,
        );
        unsafe { ffi::ImpellerPaintRelease(paint) };
        let Some(paragraph) = paragraph else {
            return Err(Status::BadScene);
        };
        // SAFETY: `paragraph` is a live object from `shape` and is stored by the caller.
        let (width, height) = unsafe {
            (
                ffi::ImpellerParagraphGetLongestLineWidth(paragraph),
                ffi::ImpellerParagraphGetHeight(paragraph),
            )
        };
        if !width.is_finite() || !height.is_finite() {
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
            return Err(Status::BadScene);
        }
        Ok(Shaped {
            width,
            height,
            paragraph,
            used: self.generation,
        })
    }

    /// Family actually passed to Impeller. A missing name falls back to the first
    /// registered face so a typo does not fail the whole frame.
    pub(crate) fn resolved_family(&self, requested: Option<&str>) -> Option<String> {
        self.resolved_face(requested)
            .and_then(|family| family.to_str().ok().map(str::to_owned))
    }

    pub(crate) fn resolved_face(&self, requested: Option<&str>) -> Option<&CString> {
        if let Some(name) = requested.filter(|name| !name.is_empty())
            && let Some(face) = self
                .fonts
                .faces
                .iter()
                .find(|face| face.family.as_bytes() == name.as_bytes())
        {
            return Some(&face.family);
        }
        let default = self.fonts.default_family.as_ref()?;
        self.fonts
            .faces
            .iter()
            .find(|face| face.family.as_c_str() == default.as_c_str())
            .map(|face| &face.family)
    }

    pub(crate) fn has_family(&self, name: &CString) -> bool {
        self.fonts
            .faces
            .iter()
            .any(|face| face.family.as_c_str() == name.as_c_str())
    }

    pub(crate) fn shape(
        &self,
        paint: Option<ffi::ImpellerPaint>,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        text_style: &TextStyle,
        spans: &[TextSpan],
    ) -> Option<ffi::ImpellerParagraph> {
        let family = self.resolved_face(text_style.family.as_deref())?;
        let family = family.as_ptr();
        let ellipsis = text_style
            .ellipsis_str()
            .and_then(|text| CString::new(text).ok());
        let layout_width = if text_style.soft_wrap || ellipsis.is_some() {
            max_width
        } else {
            max_width.max(1_000_000.0)
        };
        // SAFETY: style and the builder are released on every path. The returned
        // paragraph is owned by the caller. Font size stays in scene points; the
        // display-list scale applied before the first non-fill command scales it.
        // `family` and `ellipsis` outlive this block.
        unsafe {
            let style = ffi::ImpellerParagraphStyleNew();
            let paragraph_builder = ffi::ImpellerParagraphBuilderNew(self.context);
            if style.is_null() || paragraph_builder.is_null() {
                ffi::ImpellerParagraphStyleRelease(style);
                ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
                return None;
            }
            if let Some(paint) = paint {
                ffi::ImpellerParagraphStyleSetForeground(style, paint);
            }
            ffi::ImpellerParagraphStyleSetFontFamily(style, family);
            ffi::ImpellerParagraphStyleSetFontSize(style, font_size);
            ffi::ImpellerParagraphStyleSetFontWeight(style, impeller_weight(text_style.weight));
            ffi::ImpellerParagraphStyleSetFontStyle(
                style,
                ffi::ImpellerFontStyle::kImpellerFontStyleNormal,
            );
            ffi::ImpellerParagraphStyleSetTextAlignment(style, impeller_align(text_style.align));
            ffi::ImpellerParagraphStyleSetTextDirection(
                style,
                if text_style.direction_rtl {
                    ffi::ImpellerTextDirection::kImpellerTextDirectionRTL
                } else {
                    ffi::ImpellerTextDirection::kImpellerTextDirectionLTR
                },
            );
            if text_style.overflow != protocol::TextOverflow::Visible
                && let Some(max_lines) = text_style.max_lines.filter(|lines| *lines > 0)
            {
                ffi::ImpellerParagraphStyleSetMaxLines(style, max_lines);
            }
            if text_style.height.is_finite() && text_style.height > 0.0 {
                ffi::ImpellerParagraphStyleSetHeight(style, text_style.height);
            }
            if let Some(ellipsis) = &ellipsis {
                ffi::ImpellerParagraphStyleSetEllipsis(style, ellipsis.as_ptr());
            }
            apply_decoration(style, color, text_style.underline, text_style.line_through);
            let fallback = CString::new("UIFWSansCJK").ok();
            let cjk = fallback.as_ref().filter(|name| self.has_family(name));
            if spans.is_empty() {
                push_runs(style, paragraph_builder, family, cjk, utf8);
            } else {
                for span in spans {
                    if span.utf8.is_empty() {
                        continue;
                    }
                    let span_paint = ffi::ImpellerPaintNew();
                    if span_paint.is_null() {
                        ffi::ImpellerParagraphStyleRelease(style);
                        ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
                        return None;
                    }
                    paint_color(span_paint, span.color);
                    ffi::ImpellerParagraphStyleSetForeground(style, span_paint);
                    ffi::ImpellerParagraphStyleSetFontWeight(style, impeller_weight(span.weight));
                    apply_decoration(style, span.color, span.underline, span.line_through);
                    let span_family = self
                        .resolved_face(span.family.as_deref())
                        .map(|name| name.as_ptr())
                        .unwrap_or(family);
                    push_runs(style, paragraph_builder, span_family, cjk, &span.utf8);
                    ffi::ImpellerPaintRelease(span_paint);
                }
            }
            let paragraph =
                ffi::ImpellerParagraphBuilderBuildParagraphNew(paragraph_builder, layout_width);
            ffi::ImpellerParagraphStyleRelease(style);
            ffi::ImpellerParagraphBuilderRelease(paragraph_builder);
            if paragraph.is_null() {
                None
            } else {
                Some(paragraph)
            }
        }
    }

    pub(crate) fn store_paragraph(&mut self, key: ParaKey, shaped: Shaped) {
        if let Some(old) = self.measured.insert(key, shaped) {
            self.retire_paragraph(old.paragraph);
        }
    }

    pub(crate) fn retire_paragraph(&mut self, paragraph: ffi::ImpellerParagraph) {
        if !self.list_installed {
            // SAFETY: no display list is installed, so this is the last retain.
            unsafe { ffi::ImpellerParagraphRelease(paragraph) };
        } else {
            self.retired.push(paragraph);
        }
    }
}
