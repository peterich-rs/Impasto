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

use std::sync::Arc;

use engine::scene::{TextAlign, TextOverflow, TextStyle};
use engine::text::{
    Affinity, CanvasMetrics, CanvasTextLayout, Caret, MetricsKey, ParagraphMetrics, Shaper,
    SpanMetrics, TextSize,
};
use parley::style::{FontFamily, FontFamilyName, FontStyle, FontWeight};
use parley::{
    Alignment, AlignmentOptions, BaseDirection, FontContext, Layout, LayoutContext, LineHeight,
    StyleProperty, TextWrapMode,
};

use crate::cache::BudgetCache;
use crate::fonts::{FontLibrary, ShapeStats};
use crate::text_index::Utf8ToUtf16;

mod freeze;

use freeze::{freeze_paragraph, read_lines, size_of};

/// Brush: the logical span whose paint colors a run. `None` is the default.
type SpanBrush = Option<u32>;

const SHAPE_CAP: usize = 2048;
const SHAPE_BYTES: usize = 12 * 1024 * 1024;
const FROZEN_CAP: usize = 4096;
const FROZEN_BYTES: usize = 16 * 1024 * 1024;
const CANVAS_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHEABLE_TEXT: usize = 4096;
const CANVAS_SHAPE_CAP: usize = 1024;
const CANVAS_SHAPE_MAX_BYTES: usize = 256;

#[derive(PartialEq, Eq, Hash)]
struct CanvasShapeKey {
    text: Box<str>,
    revision: u64,
    style: TextStyle,
    size: u32,
}

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

#[derive(PartialEq, Eq, Hash)]
struct FrozenKey {
    shape: ShapeId,
    width: u32,
}

fn estimate_shape_bytes(text: &str, spans: &[SpanMetrics]) -> usize {
    // Parley layouts also retain runs, clusters and font handles.
    512 + text.len() * 96 + spans.len() * 192
}

/// Parley shaper owned by the UI queue.
pub struct ParleyShaper {
    library: FontLibrary,
    layout: LayoutContext<SpanBrush>,
    fonts: FontContext,
    shapes: BudgetCache<ShapeId, Arc<ShapedText>>,
    frozen_shapes: BudgetCache<FrozenKey, Arc<Frozen>>,
    canvas_shapes: BudgetCache<CanvasShapeKey, CanvasTextLayout>,
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
            shapes: BudgetCache::new(SHAPE_CAP, SHAPE_BYTES),
            frozen_shapes: BudgetCache::new(FROZEN_CAP, FROZEN_BYTES),
            canvas_shapes: BudgetCache::new(CANVAS_SHAPE_CAP, CANVAS_BYTES),
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

    /// Releases cached layouts after a host memory-pressure notification.
    pub fn trim(&mut self) {
        self.shapes.clear();
        self.frozen_shapes.clear();
        self.canvas_shapes.clear();
    }

    /// Registers a face under `family`. Returns `false` when the bytes are
    /// not a font. A changed mapping clears every cached layout.
    pub fn register_shared(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        match self.library.register(&mut self.fonts, bytes, family) {
            Ok(_) => {
                let invalidated =
                    self.shapes.clear() + self.frozen_shapes.clear() + self.canvas_shapes.clear();
                self.stats.registration_invalidated = self
                    .stats
                    .registration_invalidated
                    .saturating_add(u32::try_from(invalidated).unwrap_or(u32::MAX));
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

    /// Retrieves or builds the width-independent layout.
    fn shaped(&mut self, id: &ShapeId) -> (Arc<ShapedText>, bool) {
        if let Some(shaped) = self.shapes.get(id) {
            return (Arc::clone(shaped), true);
        }
        self.note_text_size(id.text.len());
        let font_size = f32::from_bits(id.font_size);
        let (layout, widths) = self.shape(&id.text, &id.style, &id.spans, font_size);
        let shaped = Arc::new(ShapedText { layout, widths });
        if id.text.len() <= MAX_CACHEABLE_TEXT {
            let evicted = self.shapes.insert(
                id.clone(),
                Arc::clone(&shaped),
                estimate_shape_bytes(&id.text, &id.spans),
            );
            self.stats.evicted = self.stats.evicted.saturating_add(evicted);
        }
        (shaped, false)
    }

    fn note_text_size(&mut self, bytes: usize) {
        let bucket = match bytes {
            0..=64 => 0,
            65..=256 => 1,
            257..=4096 => 2,
            _ => 3,
        };
        self.stats.size_buckets[bucket] = self.stats.size_buckets[bucket].saturating_add(1);
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

    /// Lines and frozen glyphs at the requested width.
    fn frozen(
        &mut self,
        key: &MetricsKey,
        shape: ShapeId,
        shaped: &ShapedText,
    ) -> (Arc<Frozen>, bool) {
        let max_width = f32::from_bits(key.max_width);
        let cache_key = FrozenKey {
            shape,
            width: key.max_width,
        };
        if let Some(frozen) = self.frozen_shapes.get(&cache_key) {
            return (Arc::clone(frozen), true);
        }
        let font_size = f32::from_bits(key.font_size);
        let (visible, spans, source_end) = visible_prefix(
            &shaped.layout,
            key,
            max_width,
            font_size,
            &mut self.layout,
            &mut self.fonts,
        );
        let mut layout = if visible == key.text {
            shaped.layout.clone()
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
        let frozen = Arc::new(Frozen {
            metrics,
            layout,
            map,
            source_end,
        });
        if key.text.len() <= MAX_CACHEABLE_TEXT {
            let evicted = self.frozen_shapes.insert(
                cache_key,
                Arc::clone(&frozen),
                estimate_shape_bytes(&visible, &spans) * 2,
            );
            self.stats.evicted = self.stats.evicted.saturating_add(evicted);
        }
        (frozen, false)
    }

    /// The broken layout for caret and word queries.
    fn with_layout<R>(
        &mut self,
        key: &MetricsKey,
        read: impl FnOnce(&Layout<SpanBrush>, &Utf8ToUtf16, u32) -> R,
    ) -> Option<R> {
        let id = shape_id(&self.library, key);
        let (shaped, _) = self.shaped(&id);
        let (frozen, _) = self.frozen(key, id, &shaped);
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

    #[cfg(debug_assertions)]
    fn unregister_dev_font(&mut self, family: &str) {
        self.library.unregister(&mut self.fonts, family);
        self.trim();
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
        let id = shape_id(&self.library, key);
        let (shaped, hit) = self.shaped(&id);
        self.stats.note_shape(hit);
        let (frozen, hit) = self.frozen(key, id, &shaped);
        self.stats.note_freeze(hit);
        Arc::clone(&frozen.metrics)
    }

    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        let (shaped, hit) = self.shaped(&shape_id(&self.library, key));
        self.stats.note_shape(hit);
        shaped.widths.min
    }

    fn min_content_metrics(&mut self, key: &MetricsKey) -> (f32, Arc<ParagraphMetrics>) {
        let id = shape_id(&self.library, key);
        let (shaped, hit) = self.shaped(&id);
        self.stats.note_shape(hit);
        let (frozen, hit) = self.frozen(key, id, &shaped);
        self.stats.note_freeze(hit);
        (shaped.widths.min, Arc::clone(&frozen.metrics))
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
        let key = (text.len() <= CANVAS_SHAPE_MAX_BYTES).then(|| CanvasShapeKey {
            text: text.into(),
            revision: self.library.revision(),
            style: style.clone(),
            size: font_size.to_bits(),
        });
        if let Some(cached) = key.as_ref().and_then(|key| self.canvas_shapes.get(key)) {
            self.stats.note_shape(true);
            self.stats.note_freeze(true);
            return Some(cached.clone());
        }
        self.note_text_size(text.len());
        self.stats.note_shape(false);
        self.stats.note_freeze(false);

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
        let result = CanvasTextLayout {
            metrics: result,
            shaped,
        };
        if let Some(key) = key {
            let evicted = self
                .canvas_shapes
                .insert(key, result.clone(), 256 + text.len() * 40);
            self.stats.evicted = self.stats.evicted.saturating_add(evicted);
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::ParleyShaper;
    use engine::scene::TextStyle;
    use engine::text::{MetricsKey, Shaper};
    use std::sync::Arc;

    #[test]
    fn canvas_text_reuses_frozen_glyphs_and_separates_size_and_direction() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        let style = TextStyle::default();
        let first = shaper.shape_canvas("交易", &style, 12.0).unwrap();
        let cached = shaper.shape_canvas("交易", &style, 12.0).unwrap();
        assert_eq!(cached, first);
        let stats = shaper.take_shape_stats();
        assert_eq!(stats.built, 1);
        assert_eq!(stats.paint_hit, 1);

        shaper.shape_canvas("交易", &style, 13.0).unwrap();
        let rtl = TextStyle {
            direction_rtl: true,
            ..style
        };
        shaper.shape_canvas("交易", &rtl, 12.0).unwrap();
        assert_eq!(shaper.take_shape_stats().built, 2);
    }

    #[test]
    fn one_use_labels_evict_cold_entries_and_keep_recent_widths() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register("Sans", crate::tests::ROBOTO));
        let narrow = key("Hot label", 14.0, 50.0, TextStyle::default());
        let wide = key("Hot label", 14.0, 150.0, TextStyle::default());
        let first = shaper.metrics(&narrow);
        let second = shaper.metrics(&wide);
        for index in 0..=super::SHAPE_CAP {
            shaper.metrics(&key(
                &format!("Cold label {index}"),
                14.0,
                150.0,
                TextStyle::default(),
            ));
            assert!(Arc::ptr_eq(&first, &shaper.metrics(&narrow)));
            assert!(Arc::ptr_eq(&second, &shaper.metrics(&wide)));
        }
        let stats = shaper.take_shape_stats();
        assert!(stats.evicted > 0);
        assert!(stats.paint_hit > stats.paint_miss);
        assert!(stats.size_buckets[0] > 0);
    }

    #[cfg(target_os = "macos")]
    fn macos_font(path: &str) -> Option<Arc<[u8]>> {
        std::fs::read(path).ok().map(Into::into)
    }

    fn key(text: &str, font_size: f32, max_width: f32, style: TextStyle) -> MetricsKey {
        engine::text::metrics_key(Arc::from(text), Arc::new(style), &[], font_size, max_width)
    }

    #[test]
    fn empty_text_keeps_the_font_box_and_italic_measures() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register("Sans", crate::tests::ROBOTO));
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
    fn freezes_glyphs_once_and_reuses_them_for_a_color_change() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register("Sans", crate::tests::ROBOTO));
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
    fn wider_width_reflows_without_reshaping() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register("Sans", crate::tests::ROBOTO));
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
    fn ellipsis_truncates_the_last_line_with_the_mark() {
        let mut shaper = ParleyShaper::new(crate::FontLibrary::new());
        assert!(shaper.register("Sans", crate::tests::ROBOTO));
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
