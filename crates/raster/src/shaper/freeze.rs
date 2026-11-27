//! Converts the visible Parley layout into owned scene data.

use std::{collections::HashMap, ops::Range, sync::Arc};

use engine::scene::{
    DecorationLine, PositionedGlyph, ShapedParagraph, ShapedRun, TextOverflow, TextStyle,
};
use engine::text::{SpanMetrics, TextLine, TextSize};
use parley::Layout;

use super::SpanBrush;
use crate::fonts::FontLibrary;
use crate::text_index::Utf8ToUtf16;

/// Converts Parley's lines into Impasto line boxes.
pub(super) fn read_lines(
    layout: &Layout<SpanBrush>,
    visible: &str,
    map: &Utf8ToUtf16,
    source_end: u32,
) -> Vec<TextLine> {
    let bytes = visible.as_bytes();
    let mut lines = Vec::with_capacity(layout.len());
    for index in 0..layout.len() {
        let Some(line) = layout.get(index) else {
            continue;
        };
        let metrics = line.metrics();
        let range = line.text_range();
        let hard = line.break_reason() == parley::BreakReason::Explicit;
        let start = map.unit_at(range.start).min(source_end);
        let trimmed = visible.get(range.clone()).map_or(range.end, |text| {
            range.start + text.trim_end_matches(['\r', '\n']).len()
        });
        let end = map.unit_at(trimmed).min(source_end);
        let end_with_break = if hard {
            let mut stop = range.end;
            if bytes.get(stop) == Some(&b'\r') {
                stop += 1;
            }
            if bytes.get(stop) == Some(&b'\n') {
                stop += 1;
            }
            map.unit_at(stop).min(source_end)
        } else {
            end
        };
        lines.push(TextLine {
            start,
            end,
            end_with_break,
            x: metrics.offset,
            y: metrics.block_min_coord,
            w: metrics.advance,
            h: metrics.line_height,
            hard_break: hard,
            baseline: metrics.baseline,
        });
    }
    lines
}

pub(super) fn size_of(
    layout: &Layout<SpanBrush>,
    lines: &[TextLine],
    max_width: f32,
    style: &TextStyle,
) -> TextSize {
    let widest = layout.width();
    let width = match style.overflow {
        TextOverflow::Visible => widest,
        _ if lines.len() == 1 && widest > max_width => widest,
        _ => widest.min(max_width),
    };
    TextSize {
        width,
        height: layout.height(),
    }
}

/// Freezes glyphs, runs, and decoration bars into scene data.
pub(super) fn freeze_paragraph(
    layout: &Layout<SpanBrush>,
    library: &FontLibrary,
    spans: &[SpanMetrics],
) -> Option<Arc<ShapedParagraph>> {
    let mut runs: Vec<ShapedRun> = Vec::new();
    let mut decorations: Vec<DecorationLine> = Vec::new();
    let mut faces: HashMap<(*const u8, u32), Arc<engine::scene::FontFaceId>> = HashMap::new();
    for index in 0..layout.len() {
        let line = layout.get(index)?;
        let metrics = line.metrics();
        let baseline = metrics.baseline;
        let line_text = line.text_range();
        let extents = line_span_extents(&line, spans, &line_text);
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let style = run.style();
            let data = run.run().font();
            let key = (data.data.data().as_ptr(), data.index);
            let face = match faces.get(&key) {
                Some(id) => Arc::clone(id),
                None => {
                    let id = library.resolve(data);
                    faces.insert(key, Arc::clone(&id));
                    id
                }
            };
            let glyphs: Vec<PositionedGlyph> = run
                .positioned_glyphs()
                .map(|glyph| PositionedGlyph {
                    id: u16::try_from(glyph.id).unwrap_or(0),
                    x: glyph.x,
                    y: glyph.y,
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            let synthesis = run.run().synthesis();
            let font_metrics = run.run().font_metrics();
            let span_index = style.brush;
            let span_bounds = match span_index {
                Some(index) => extents
                    .get(index as usize)
                    .copied()
                    .unwrap_or((run.offset(), run.offset() + run.advance())),
                None => (run.offset(), run.offset() + run.advance()),
            };
            let span_bounds = [
                span_bounds.0,
                baseline - font_metrics.ascent,
                span_bounds.1 - span_bounds.0,
                font_metrics.ascent + font_metrics.descent,
            ];
            let size = run.run().font_size();
            let x = run.offset();
            let advance = run.advance();
            runs.push(ShapedRun {
                face,
                font_size: size,
                normalized_coords: run
                    .run()
                    .normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect(),
                embolden: synthesis.embolden(),
                skew_degrees: synthesis.skew(),
                span_index,
                span_bounds,
                glyphs: glyphs.into(),
            });
            if let Some(decoration) = &style.underline {
                decorations.push(decoration_line(
                    x,
                    advance,
                    baseline,
                    -decoration.offset.unwrap_or(font_metrics.underline_offset),
                    decoration.size.unwrap_or(font_metrics.underline_size),
                    span_index,
                    span_bounds,
                ));
            }
            if let Some(decoration) = &style.strikethrough {
                decorations.push(decoration_line(
                    x,
                    advance,
                    baseline,
                    -decoration
                        .offset
                        .unwrap_or(font_metrics.strikethrough_offset),
                    decoration.size.unwrap_or(font_metrics.strikethrough_size),
                    span_index,
                    span_bounds,
                ));
            }
        }
    }
    if runs.is_empty() {
        return None;
    }
    Some(Arc::new(ShapedParagraph {
        size: [layout.width(), layout.height()],
        runs: runs.into(),
        decorations: decorations.into(),
    }))
}

/// Horizontal extent `[x0, x1]` of each span on one line, from the glyph
/// runs whose text range overlaps the span. Gradients sample this box, so
/// every run of one span on one line shares the same box and a fallback
/// font does not restart the gradient.
fn line_span_extents(
    line: &parley::Line<'_, SpanBrush>,
    spans: &[SpanMetrics],
    line_text: &Range<usize>,
) -> Vec<(f32, f32)> {
    let mut extents = vec![(f32::INFINITY, f32::NEG_INFINITY); spans.len()];
    let mut offsets = Vec::with_capacity(spans.len());
    let mut offset = 0usize;
    for span in spans {
        offsets.push(offset);
        offset += span.utf8.len();
    }
    for item in line.items() {
        let parley::PositionedLayoutItem::GlyphRun(run) = item else {
            continue;
        };
        let range = run.run().text_range();
        if range.start >= range.end {
            continue;
        }
        let (x0, x1) = (run.offset(), run.offset() + run.advance());
        for (index, (start, extent)) in offsets.iter().zip(extents.iter_mut()).enumerate() {
            let from = (*start).max(line_text.start);
            let to = (start + spans[index].utf8.len()).min(line_text.end);
            if from >= to || range.end <= from || range.start >= to {
                continue;
            }
            extent.0 = extent.0.min(x0);
            extent.1 = extent.1.max(x1);
        }
    }
    extents
}

fn decoration_line(
    x: f32,
    width: f32,
    baseline: f32,
    offset: f32,
    thickness: f32,
    span_index: Option<u32>,
    span_bounds: [f32; 4],
) -> DecorationLine {
    DecorationLine {
        rect: [x, baseline + offset - thickness * 0.5, width, thickness],
        span_index,
        span_bounds,
    }
}
