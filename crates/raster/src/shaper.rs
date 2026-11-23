//! Parley-backed [`Shaper`] for the UI queue.
//!
//! One instance lives on the UI thread. It shapes with Parley, freezes the
//! glyphs into [`engine::scene::ShapedParagraph`], and keeps a two-level
//! cache: the shaping result (independent of width and color) and the
//! broken, frozen result (per width). A color change reuses both.
//!
//! Impasto's truncation rules (`max_lines`, a custom ellipsis, and no soft
//! wrap) are applied here rather than by Parley: the shaper decides which
//! source prefix stays visible, then shapes that prefix with the same span
//! styles. Line boxes, carets, and glyphs all come from the visible layout.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use engine::scene::{
    DecorationLine, PositionedGlyph, ShapedParagraph, ShapedRun, TextAlign, TextOverflow, TextStyle,
};
use engine::text::{
    Affinity, CanvasMetrics, CanvasTextLayout, Caret, MetricsKey, ParagraphMetrics, Shaper,
    SpanMetrics, TextLine, TextSize,
};
use parley::style::{FontFamily, FontFamilyName, FontStyle, FontWeight};
use parley::{
    Alignment, AlignmentOptions, BaseDirection, FontContext, Layout, LayoutContext, LineHeight,
    StyleProperty, TextWrapMode,
};

use crate::fonts::{FontLibrary, ShapeStats};
use crate::text_index::Utf8ToUtf16;

/// Brush: the logical span whose paint colors a run. `None` is the default.
type SpanBrush = Option<u32>;

const SHAPE_CAP: usize = 2048;
const FROZEN_CAP: usize = 16;

/// Geometry a shaping cache entry depends on. Width and color are excluded.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapeId {
    revision: u64,
    text: Arc<str>,
    style: Arc<TextStyle>,
    spans: Arc<[SpanMetrics]>,
    font_size: u32,
}

/// Shaped but unbroken text, plus its intrinsic widths.
struct ShapedText {
    layout: Layout<SpanBrush>,
    widths: parley::ContentWidths,
}

/// One broken layout at one width, with its frozen result.
struct Frozen {
    metrics: Arc<ParagraphMetrics>,
    /// The visible layout, kept for caret and word queries at this width.
    layout: Layout<SpanBrush>,
    /// Visible text, so byte offsets map to UTF-16 units of the source.
    map: Utf8ToUtf16,
    source_end: u32,
}

struct ShapeEntry {
    shaped: ShapedText,
    frozen: HashMap<u32, Frozen>,
}

/// Parley shaper owned by the UI queue.
pub struct ParleyShaper {
    library: FontLibrary,
    layout: LayoutContext<SpanBrush>,
    fonts: FontContext,
    shapes: HashMap<ShapeId, ShapeEntry>,
    stats: ShapeStats,
}

impl std::fmt::Debug for ParleyShaper {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParleyShaper")
            .field("shapes", &self.shapes.len())
            .finish_non_exhaustive()
    }
}

impl ParleyShaper {
    pub fn new(library: FontLibrary) -> Self {
        Self {
            library,
            layout: LayoutContext::new(),
            fonts: FontContext::new(),
            shapes: HashMap::new(),
            stats: ShapeStats::default(),
        }
    }

    /// The library this shaper resolves faces through. Share it with the
    /// raster so a frozen run's face ID finds its bytes on that queue.
    pub fn library(&self) -> FontLibrary {
        self.library.clone()
    }

    /// Counters since the last take.
    pub fn take_shape_stats(&mut self) -> ShapeStats {
        std::mem::take(&mut self.stats)
    }

    /// Registers a face under `family`. Returns `false` when the bytes are
    /// not a font. A changed mapping clears every cached layout.
    pub fn register_shared(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        match self.library.register(&mut self.fonts, bytes, family) {
            Ok(_) => {
                self.shapes.clear();
                true
            }
            Err(status) => {
                tracing::warn!(target: "impasto::text", %family, %status, "font registration failed");
                false
            }
        }
    }

    pub fn register(&mut self, family: &str, bytes: &[u8]) -> bool {
        self.register_shared(family, Arc::from(bytes))
    }

    /// Cached shaping, or a fresh one. Does not break lines. Returns
    /// whether the shaping cache hit.
    fn ensure_shaped(&mut self, key: &MetricsKey) -> bool {
        let id = shape_id(&self.library, key);
        if self.shapes.contains_key(&id) {
            return true;
        }
        let font_size = f32::from_bits(key.font_size);
        let (layout, widths) = self.shape(&key.text, &key.style, &key.spans, font_size);
        if self.shapes.len() >= SHAPE_CAP {
            self.shapes.clear();
        }
        self.shapes.insert(
            id,
            ShapeEntry {
                shaped: ShapedText { layout, widths },
                frozen: HashMap::new(),
            },
        );
        false
    }

    /// Shapes without line breaks. Whitespace is preserved so the line
    /// breaker sees the original text.
    fn shape(
        &mut self,
        text: &str,
        style: &TextStyle,
        spans: &[SpanMetrics],
        font_size: f32,
    ) -> (Layout<SpanBrush>, parley::ContentWidths) {
        let family = style.family.clone().unwrap_or_else(|| "Sans".to_owned());
        // Layout units are scene points. The font size is a style property;
        // the raster applies the device pixel ratio when it scales glyphs.
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, text, 1.0, false);
        builder.set_base_direction(if style.direction_rtl {
            BaseDirection::Rtl
        } else {
            BaseDirection::Ltr
        });
        builder.push_default(StyleProperty::FontFamily(font_families(&family)));
        builder.push_default(StyleProperty::FontSize(font_size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(f32::from(
            style.weight,
        ))));
        builder.push_default(StyleProperty::FontStyle(match style.slant {
            engine::scene::FontSlant::Normal => FontStyle::Normal,
            engine::scene::FontSlant::Italic => FontStyle::Italic,
        }));
        builder.push_default(StyleProperty::LineHeight(
            if style.height.is_finite() && style.height > 0.0 {
                LineHeight::FontSizeRelative(style.height)
            } else {
                LineHeight::MetricsRelative(1.0)
            },
        ));
        builder.push_default(StyleProperty::Underline(style.underline));
        builder.push_default(StyleProperty::Strikethrough(style.line_through));
        builder.push_default(StyleProperty::TextWrapMode(if style.soft_wrap {
            TextWrapMode::Wrap
        } else {
            TextWrapMode::NoWrap
        }));
        builder.push_default(StyleProperty::Brush(SpanBrush::None));
        for (index, span) in spans.iter().enumerate() {
            let start = span_start(spans, index);
            let range = start..start + span.utf8.len();
            if !span.family.is_empty() {
                builder.push(
                    StyleProperty::FontFamily(font_families(&span.family)),
                    range.clone(),
                );
            }
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(f32::from(span.weight))),
                range.clone(),
            );
            builder.push(StyleProperty::Underline(span.underline), range.clone());
            builder.push(
                StyleProperty::Strikethrough(span.line_through),
                range.clone(),
            );
            builder.push(StyleProperty::Brush(u32::try_from(index).ok()), range);
        }
        let layout = builder.build(text);
        let mut widths = layout.calculate_content_widths();
        if text
            .chars()
            .any(|ch| matches!(ch as u32, 0x0590..=0x08ff | 0xfb1d..=0xfeff))
        {
            let mut probe = layout.clone();
            probe.break_all_lines(Some(0.0));
            widths.min = probe.width();
            probe.break_all_lines(None);
            widths.max = probe.width();
        }
        (layout, widths)
    }

    /// Lines and frozen glyphs for one width, cached per width. Returns
    /// whether the per-width cache hit.
    fn frozen(&mut self, key: &MetricsKey) -> (Arc<ParagraphMetrics>, bool) {
        let max_width = f32::from_bits(key.max_width);
        self.ensure_shaped(key);
        let id = shape_id(&self.library, key);
        if let Some(frozen) = self
            .shapes
            .get(&id)
            .and_then(|entry| entry.frozen.get(&key.max_width))
        {
            return (Arc::clone(&frozen.metrics), true);
        }
        let font_size = f32::from_bits(key.font_size);
        let Some(entry) = self.shapes.get_mut(&id) else {
            unreachable!("ensure_shaped inserted the entry")
        };
        let (visible, spans, source_end) = visible_prefix(
            &entry.shaped.layout,
            key,
            max_width,
            font_size,
            &mut self.layout,
            &mut self.fonts,
        );
        let mut layout = if visible == key.text {
            entry.shaped.layout.clone()
        } else {
            let (layout, _) = self.shape(&visible, &key.style, &spans, font_size);
            self.stats.built = self.stats.built.saturating_add(1);
            layout
        };
        layout.set_alignment(
            physical_alignment(key.style.align),
            AlignmentOptions {
                align_when_overflowing: true,
                last_line_alignment: None,
            },
        );
        layout.break_all_lines(width_constraint(max_width));
        let map = Utf8ToUtf16::new(&visible);
        let source_end = map.unit_at(source_end);
        let lines = read_lines(&layout, &visible, &map, source_end);
        let shaped = freeze_paragraph(&layout, &self.library, &spans);
        let size = size_of(&layout, &lines, max_width, &key.style);
        let baseline = lines.first().map_or(0.0, |line| line.baseline);
        let metrics = Arc::new(match shaped {
            Some(shaped) => ParagraphMetrics::shaped(size, Arc::from(lines), baseline, shaped),
            None => ParagraphMetrics::with_baseline(size, Arc::from(lines), baseline),
        });
        let Some(entry) = self.shapes.get_mut(&id) else {
            unreachable!("ensure_shaped inserted the entry")
        };
        if entry.frozen.len() >= FROZEN_CAP {
            entry.frozen.clear();
        }
        entry.frozen.insert(
            key.max_width,
            Frozen {
                metrics: Arc::clone(&metrics),
                layout,
                map,
                source_end,
            },
        );
        (metrics, false)
    }

    /// The broken layout for caret and word queries.
    fn with_layout<R>(
        &mut self,
        key: &MetricsKey,
        read: impl FnOnce(&Layout<SpanBrush>, &Utf8ToUtf16, u32) -> R,
    ) -> Option<R> {
        self.frozen(key);
        let entry = self.shapes.get(&shape_id(&self.library, key))?;
        let frozen = entry.frozen.get(&key.max_width)?;
        Some(read(&frozen.layout, &frozen.map, frozen.source_end))
    }
}

fn shape_id(library: &FontLibrary, key: &MetricsKey) -> ShapeId {
    ShapeId {
        revision: library.revision(),
        text: Arc::clone(&key.text),
        style: Arc::clone(&key.style),
        spans: Arc::clone(&key.spans),
        font_size: key.font_size,
    }
}

/// Byte offset where span `index` starts in the concatenated text.
fn span_start(spans: &[SpanMetrics], index: usize) -> usize {
    spans[..index].iter().map(|span| span.utf8.len()).sum()
}

fn physical_alignment(align: TextAlign) -> Alignment {
    match align {
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
    }
}

fn font_families(family: &str) -> FontFamily<'_> {
    FontFamily::List(
        vec![
            FontFamilyName::named(family),
            FontFamilyName::named("SansCJK"),
            FontFamilyName::Generic(parley::style::GenericFamily::SansSerif),
        ]
        .into(),
    )
}

fn width_constraint(max_width: f32) -> Option<f32> {
    (max_width.is_finite() && max_width < engine::text::SINGLE_LINE_MAX)
        .then_some(max_width.max(0.0))
}

fn visible_prefix(
    unbroken: &Layout<SpanBrush>,
    key: &MetricsKey,
    max_width: f32,
    font_size: f32,
    lcx: &mut LayoutContext<SpanBrush>,
    fcx: &mut FontContext,
) -> (Arc<str>, Arc<[SpanMetrics]>, usize) {
    let text = &key.text;
    let spans = &key.spans;
    let style = &key.style;
    let full = || (Arc::clone(text), Arc::clone(spans), text.len());
    if style.overflow == TextOverflow::Visible || text.is_empty() {
        return full();
    }
    let mut probe = unbroken.clone();
    probe.break_all_lines(width_constraint(max_width));
    let limit = style.max_lines.filter(|&max| max > 0).unwrap_or(u32::MAX) as usize;
    let index = limit.min(probe.len()).saturating_sub(1);
    let Some(last) = probe.get(index) else {
        return full();
    };
    let mark = style.ellipsis_str();
    if probe.len() <= limit && (mark.is_none() || last.metrics().advance <= max_width) {
        return full();
    }

    let line_start = last.text_range().start;
    let mut end = last.text_range().end;
    if let Some(mark) = mark {
        let available = max_width - mark_width(mark, style, font_size, lcx, fcx);
        let mut advance = last.metrics().advance;
        // Parley supplies clusters in visual order; cut at a source boundary
        // so RTL tails, ligatures and emoji sequences stay intact.
        let mut clusters: Vec<_> = last.runs().flat_map(|run| run.visual_clusters()).collect();
        clusters.sort_unstable_by_key(|cluster| std::cmp::Reverse(cluster.text_range().start));
        for cluster in clusters {
            if advance <= available {
                break;
            }
            advance -= cluster.advance();
            end = cluster.text_range().start.max(line_start);
        }
    }
    narrow(text, spans, end, mark)
}

fn narrow(
    text: &str,
    spans: &[SpanMetrics],
    end: usize,
    mark: Option<&str>,
) -> (Arc<str>, Arc<[SpanMetrics]>, usize) {
    let kept = text.get(..end).unwrap_or(text).trim_end();
    let cut = kept.len();
    let mut out = Vec::new();
    let mut remaining = cut;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(span.utf8.len());
        let mut span = span.clone();
        if let Some(prefix) = span.utf8.get(..take) {
            span.utf8 = Arc::from(prefix);
        }
        remaining -= take;
        out.push(span);
    }
    let mut visible = kept.to_owned();
    if let Some(mark) = mark {
        visible.push_str(mark);
        if let Some(last) = out.last_mut() {
            let mut value = last.utf8.to_string();
            value.push_str(mark);
            last.utf8 = Arc::from(value);
        }
    }
    (Arc::from(visible), Arc::from(out), cut)
}

/// Width of the ellipsis mark shaped alone with the paragraph style.
fn mark_width(
    mark: &str,
    style: &TextStyle,
    font_size: f32,
    lcx: &mut LayoutContext<SpanBrush>,
    fcx: &mut FontContext,
) -> f32 {
    let family = style.family.clone().unwrap_or_else(|| "Sans".to_owned());
    let mut builder = lcx.ranged_builder(fcx, mark, 1.0, false);
    builder.push_default(StyleProperty::FontFamily(font_families(&family)));
    builder.push_default(StyleProperty::FontSize(font_size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(f32::from(
        style.weight,
    ))));
    builder.push_default(StyleProperty::FontStyle(match style.slant {
        engine::scene::FontSlant::Normal => FontStyle::Normal,
        engine::scene::FontSlant::Italic => FontStyle::Italic,
    }));
    builder.push_default(StyleProperty::Brush(SpanBrush::None));
    let mut layout = builder.build(mark);
    layout.break_all_lines(None);
    layout.width()
}

/// Converts Parley's lines into Impasto line boxes.
fn read_lines(
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

fn size_of(
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
fn freeze_paragraph(
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

fn empty_metrics(style: &TextStyle, font_size: f32) -> ParagraphMetrics {
    ParagraphMetrics::with_baseline(
        TextSize {
            width: 0.0,
            height: engine::text::resolved_line_height(style, font_size, font_size),
        },
        Arc::from([]),
        0.0,
    )
}

fn empty_caret(font_size: f32, affinity: Affinity) -> Caret {
    Caret {
        unit: 0,
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: font_size.max(0.0),
        affinity,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the bounds originate from Parley f32 layout coordinates"
)]
fn caret_from(bounds: parley::BoundingBox, unit: u32, affinity: Affinity) -> Caret {
    Caret {
        unit,
        x: bounds.x0 as f32,
        y: bounds.y0 as f32,
        w: 1.0,
        h: (bounds.y1 - bounds.y0) as f32,
        affinity,
    }
}

impl Shaper for ParleyShaper {
    fn measure_cache_counts(&self) -> (u32, u32) {
        (self.stats.measure_hit, self.stats.measure_miss)
    }

    fn register_font(&mut self, family: &str, bytes: &[u8]) -> bool {
        ParleyShaper::register(self, family, bytes)
    }

    fn register_shared_font(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        ParleyShaper::register_shared(self, family, bytes)
    }

    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        let font_size = f32::from_bits(key.font_size);
        if key.text.is_empty() {
            return Arc::new(empty_metrics(&key.style, font_size));
        }
        let max_width = f32::from_bits(key.max_width);
        if !max_width.is_finite() || max_width <= 0.0 {
            // A collapsed box has no line to draw but keeps a font box.
            return Arc::new(ParagraphMetrics::with_baseline(
                TextSize {
                    width: 0.0,
                    height: engine::text::resolved_line_height(&key.style, font_size, font_size),
                },
                Arc::from([]),
                0.0,
            ));
        }
        let hit = self.ensure_shaped(key);
        self.stats.note_shape(hit);
        let (metrics, hit) = self.frozen(key);
        self.stats.note_freeze(hit);
        metrics
    }

    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        let hit = self.ensure_shaped(key);
        self.stats.note_shape(hit);
        self.shapes
            .get(&shape_id(&self.library, key))
            .map_or(0.0, |entry| entry.shaped.widths.min)
    }

    fn min_content_metrics(&mut self, key: &MetricsKey) -> (f32, Arc<ParagraphMetrics>) {
        let hit = self.ensure_shaped(key);
        self.stats.note_shape(hit);
        let min = self
            .shapes
            .get(&shape_id(&self.library, key))
            .map_or(0.0, |entry| entry.shaped.widths.min);
        let (metrics, hit) = self.frozen(key);
        self.stats.note_freeze(hit);
        (min, metrics)
    }

    fn touch(&mut self, _key: &MetricsKey) {}

    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        if key.text.is_empty() {
            return empty_caret(font_size, affinity);
        }
        self.with_layout(key, |layout, map, source_end| {
            let unit = unit.min(source_end);
            let cursor = parley::Cursor::from_byte_index(
                layout,
                map.byte_at(unit),
                if affinity == Affinity::Upstream {
                    parley::Affinity::Upstream
                } else {
                    parley::Affinity::Downstream
                },
            );
            caret_from(cursor.geometry(layout, 1.0), unit, affinity)
        })
        .unwrap_or_else(|| empty_caret(font_size, affinity))
    }

    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        if key.text.is_empty() {
            return empty_caret(font_size, Affinity::Downstream);
        }
        self.with_layout(key, |layout, map, source_end| {
            let cursor = parley::Cursor::from_point(layout, x, y);
            let unit = map.unit_at(cursor.index()).min(source_end);
            let affinity = if cursor.affinity() == parley::Affinity::Upstream {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            };
            caret_from(cursor.geometry(layout, 1.0), unit, affinity)
        })
        .unwrap_or_else(|| empty_caret(font_size, Affinity::Downstream))
    }

    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32) {
        if key.text.is_empty() {
            return (0, 0);
        }
        self.with_layout(key, |layout, map, source_end| {
            let unit = unit.min(source_end);
            let cursor = parley::Cursor::from_byte_index(
                layout,
                map.byte_at(unit),
                parley::Affinity::Downstream,
            );
            let Some(cluster) = cursor.downstream_cluster(layout) else {
                return (unit, unit);
            };
            let mut first = cluster;
            let mut last = cluster;
            if !last.is_space_or_nbsp() {
                while !first.is_word_boundary() {
                    let Some(previous) = first.previous_logical() else {
                        break;
                    };
                    first = previous;
                }
                while let Some(next) = last.next_logical() {
                    if next.is_word_boundary() {
                        break;
                    }
                    last = next;
                }
            }
            (
                map.unit_at(first.text_range().start).min(source_end),
                map.unit_at(last.text_range().end).min(source_end),
            )
        })
        .unwrap_or((0, 0))
    }

    fn canvas_metrics(
        &mut self,
        text: &str,
        style: &TextStyle,
        font_size: f32,
    ) -> Option<CanvasMetrics> {
        self.shape_canvas(text, style, font_size)
            .map(|layout| layout.metrics)
    }

    fn shape_canvas(
        &mut self,
        text: &str,
        style: &TextStyle,
        font_size: f32,
    ) -> Option<CanvasTextLayout> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        // Canvas text is one unbreakable line. An empty string still reports
        // the font box, measured on a space, and no glyphs.
        let probe = if text.is_empty() { " " } else { text };
        let (mut layout, _) = self.shape(probe, style, &[], font_size);
        layout.break_all_lines(None);
        let line = layout.get(0)?;
        let metrics = line.metrics();
        // Parley's baseline is measured from the line top. The font box
        // around the alphabetic baseline is the ascent above and the
        // remainder of the line height below.
        let result = CanvasMetrics {
            width: if text.is_empty() {
                0.0
            } else {
                metrics.advance
            },
            ascent: metrics.baseline,
            descent: metrics.line_height - metrics.baseline,
        };
        let shaped = if text.is_empty() {
            None
        } else {
            freeze_paragraph(&layout, &self.library, &[])
        };
        Some(CanvasTextLayout {
            metrics: result,
            shaped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ParleyShaper;
    use engine::scene::TextStyle;
    use engine::text::{MetricsKey, Shaper};
    use std::sync::Arc;

    fn macos_font(path: &str) -> Option<Arc<[u8]>> {
        std::fs::read(path).ok().map(Into::into)
    }

    fn key(text: &str, font_size: f32, max_width: f32, style: TextStyle) -> MetricsKey {
        engine::text::metrics_key(Arc::from(text), Arc::new(style), &[], font_size, max_width)
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn empty_text_keeps_the_font_box_and_italic_measures() {
        let Some(font) = macos_font("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", font));
        let style = TextStyle::default();
        let Some(empty) = shaper.canvas_metrics("", &style, 16.0) else {
            panic!("empty metrics");
        };
        let Some(letter) = shaper.canvas_metrics("M", &style, 16.0) else {
            panic!("letter metrics");
        };
        assert_eq!(empty.width, 0.0);
        assert!(empty.ascent > 0.0);
        assert!(letter.width > 0.0);
        assert!(shaper.canvas_metrics("M", &style, 0.0).is_none());
        assert!(shaper.canvas_metrics("M", &style, f32::NAN).is_none());
        let italic = TextStyle {
            slant: engine::scene::FontSlant::Italic,
            ..TextStyle::default()
        };
        let Some(slanted) = shaper.canvas_metrics("M", &italic, 16.0) else {
            panic!("italic metrics");
        };
        assert!(slanted.width > 0.0);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn freezes_glyphs_once_and_reuses_them_for_a_color_change() {
        let Some(font) = macos_font("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", font));
        let first = shaper.metrics(&key("Hi", 16.0, 100.0, TextStyle::default()));
        let shaped = first.shaped.as_ref().expect("glyphs");
        assert!(!shaped.runs.is_empty());
        assert!(shaped.size[0] > 0.0 && shaped.size[1] > 0.0);
        // Every glyph names a face the library can read back.
        for run in &shaped.runs {
            assert!(
                shaper.library().has_face(&run.face),
                "face {} must be in the library",
                run.face.0
            );
        }
        let stats = shaper.take_shape_stats();
        assert!(stats.built >= 1);
        let again = shaper.metrics(&key("Hi", 16.0, 100.0, TextStyle::default()));
        let stats = shaper.take_shape_stats();
        assert_eq!(stats.built, 0, "the same geometry does not shape again");
        assert!(stats.measure_hit >= 1);
        assert!(stats.paint_hit >= 1);
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn wider_width_reflows_without_reshaping() {
        let Some(font) = macos_font("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", font));
        let style = TextStyle::default();
        let narrow = shaper.metrics(&key("Hello World", 16.0, 40.0, style.clone()));
        let wide = shaper.metrics(&key("Hello World", 16.0, 400.0, style));
        assert!(narrow.size.height > wide.size.height, "narrow wraps");
        let stats = shaper.take_shape_stats();
        assert_eq!(stats.built, 1, "shaping is shared across widths");
        assert_eq!(stats.paint_miss, 2);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn carets_and_words_use_utf16_units() {
        let Some(font) = macos_font("/System/Library/Fonts/Hiragino Sans GB.ttc") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", font));
        let key = key("a中b", 16.0, 400.0, TextStyle::default());
        let caret = shaper.caret_at_unit(&key, 3, engine::text::Affinity::Downstream);
        assert_eq!(caret.unit, 3);
        assert!(caret.x > 0.0 && caret.h > 0.0);
        let point = shaper.caret_at_point(&key, caret.x + 0.01, 2.0);
        assert_eq!(point.unit, 3);
        let (start, end) = shaper.word_range(&key, 1);
        assert!(start <= 1 && end >= 2, "{start}..{end}");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn ellipsis_truncates_the_last_line_with_the_mark() {
        let Some(font) = macos_font("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", font));
        let style = TextStyle {
            max_lines: Some(1),
            overflow: engine::scene::TextOverflow::Ellipsis,
            ..TextStyle::default()
        };
        let metrics = shaper.metrics(&key("Hello World", 16.0, 50.0, style));
        assert_eq!(metrics.lines.len(), 1);
        assert!(metrics.size.width <= 50.5, "{:?}", metrics.size);
        let shaped = metrics.shaped.as_ref().expect("glyphs");
        let glyphs: usize = shaped.runs.iter().map(|run| run.glyphs.len()).sum();
        assert!(glyphs < 11, "{glyphs} glyphs");
        assert!(glyphs >= 2, "{glyphs} glyphs");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn cjk_and_emoji_fall_back_and_stay_in_the_library() {
        let Some(latin) = macos_font("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register_shared("Sans", latin));
        let metrics = shaper.metrics(&key("港股 12.4 😀", 16.0, 400.0, TextStyle::default()));
        let shaped = metrics.shaped.as_ref().expect("glyphs");
        assert!(shaped.runs.len() >= 2, "fallback splits runs");
        for run in &shaped.runs {
            assert!(shaper.library().has_face(&run.face));
        }
    }
}
