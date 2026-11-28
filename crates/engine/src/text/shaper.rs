use std::sync::Arc;

/// Text, size, width, color, and style that identify one measured paragraph.
#[derive(Clone, Copy)]
pub struct ParagraphQuery<'a> {
    pub utf8: &'a str,
    pub font_size: f32,
    pub max_width: f32,
    pub color: [u8; 4],
    pub style: &'a crate::scene::TextStyle,
}

/// Paragraph size in scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSize {
    pub width: f32,
    pub height: f32,
}

/// Which side of a soft line break a collapsed caret occupies.
///
/// The same UTF-16 offset is the end of the previous line ([`Affinity::Upstream`])
/// and the start of the next line ([`Affinity::Downstream`]). Hard breaks have
/// distinct offsets, so affinity does not apply to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Affinity {
    #[default]
    Downstream,
    Upstream,
}

/// A caret in paragraph space. `unit` is a UTF-16 index, the same unit as
/// editing state. `x` and `y` are relative to the paragraph origin. `w` is the
/// caret thickness in scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub unit: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub affinity: Affinity,
}

/// One laid-out line. Offsets are UTF-16 code units into the paragraph.
///
/// `end` excludes a trailing hard break. `end_with_break` includes it. A soft
/// wrap has `end == end_with_break` and `hard_break == false`, so the next
/// line starts at the same offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLine {
    pub start: u32,
    pub end: u32,
    pub end_with_break: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub hard_break: bool,
    /// Distance from the top of the line box to the baseline.
    pub baseline: f32,
}

/// Line limit used when the parent does not set a width.
///
/// Measurement and the later text command both use this value, so a single
/// line is shaped the same way when it is drawn.
pub const SINGLE_LINE_MAX: f32 = 1_000_000.0;

/// Color-free identity of one paragraph. `font_size` and `max_width` are the
/// IEEE bits of the scene-point values, so two widths that compare equal as
/// `f32` share a key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MetricsKey {
    pub text: Arc<str>,
    pub style: Arc<crate::scene::TextStyle>,
    pub spans: Arc<[SpanMetrics]>,
    pub font_size: u32,
    pub max_width: u32,
}

/// One run, without its color. Underline and strike change the shaped run, so
/// they stay on this key even though the paint color does not.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpanMetrics {
    pub utf8: Arc<str>,
    pub family: Arc<str>,
    pub weight: u16,
    pub underline: bool,
    pub line_through: bool,
}

/// Width and font box for one canvas text run.
///
/// `ascent` and `descent` are the font bounding box around the alphabetic
/// baseline. An empty run still reports the box and a width of zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// Laid-out size and lines. Shared by every color of the same [`MetricsKey`].
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphMetrics {
    pub size: TextSize,
    pub lines: Arc<[TextLine]>,
    /// Glyphs frozen at the final constraint. `None` for a shaper that only
    /// measures, or for empty text.
    pub shaped: Option<Arc<crate::scene::ShapedParagraph>>,
    /// Distance from the paragraph origin to the first line's baseline.
    /// Zero when the paragraph has no lines.
    first_baseline: f32,
}

impl ParagraphMetrics {
    pub fn first_baseline(&self) -> f32 {
        self.first_baseline
    }

    /// Metrics without frozen glyphs. A measuring-only shaper uses this.
    pub fn with_baseline(size: TextSize, lines: Arc<[TextLine]>, first_baseline: f32) -> Self {
        Self {
            size,
            lines,
            shaped: None,
            first_baseline,
        }
    }

    pub fn shaped(
        size: TextSize,
        lines: Arc<[TextLine]>,
        first_baseline: f32,
        shaped: Arc<crate::scene::ShapedParagraph>,
    ) -> Self {
        Self {
            size,
            lines,
            shaped: Some(shaped),
            first_baseline,
        }
    }
}

/// Width, font box, and glyphs for one canvas text run.
///
/// `shaped` is `None` for a shaper that cannot draw; the caller then has
/// metrics but the raster skips the glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasTextLayout {
    pub metrics: CanvasMetrics,
    pub shaped: Option<Arc<crate::scene::ShapedParagraph>>,
}

/// Line box height. A positive `style.height` is a multiple of `font_size`.
/// Otherwise the font's own line box (`font_line`) is used.
pub fn resolved_line_height(
    style: &crate::scene::TextStyle,
    font_size: f32,
    font_line: f32,
) -> f32 {
    if style.height.is_finite() && style.height > 0.0 {
        font_size * style.height
    } else {
        font_line
    }
}

/// Builds the keys a text node retains. Empty span slices are a single run.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::scene::{Command, DrawList};

    /// Source text of one text command frozen by the [`super::MonoShaper`].
    pub fn mono_text(shaped: &crate::scene::ShapedParagraph) -> String {
        super::MonoShaper::text_of(shaped).unwrap_or_default()
    }

    /// Text of every text command in a recorded list, in order.
    pub fn texts(list: &DrawList) -> Vec<String> {
        list.commands
            .iter()
            .filter_map(|command| match command {
                Command::Text { shaped, .. } => shaped.as_deref().map(mono_text),
                _ => None,
            })
            .collect()
    }
}

/// Builds the geometry key a text node retains. Color is not part of the key:
/// a color change reuses the lines and the frozen glyphs.
pub fn metrics_key(
    text: Arc<str>,
    style: Arc<crate::scene::TextStyle>,
    spans: &[crate::scene::TextSpan],
    font_size: f32,
    max_width: f32,
) -> MetricsKey {
    let span_metrics: Vec<SpanMetrics> = spans
        .iter()
        .map(|span| SpanMetrics {
            utf8: Arc::from(span.utf8.as_str()),
            family: Arc::from(span.family.as_deref().unwrap_or("")),
            weight: span.weight,
            underline: span.underline,
            line_through: span.line_through,
        })
        .collect();
    MetricsKey {
        text,
        style,
        spans: Arc::from(span_metrics),
        font_size: font_size.to_bits(),
        max_width: max_width.to_bits(),
    }
}

/// Foreground paints of a paragraph's logical spans, in order.
pub fn span_paints(spans: &[crate::scene::TextSpan]) -> Arc<[crate::scene::SpanPaint]> {
    spans.iter().map(crate::scene::TextSpan::paint).collect()
}

/// Measures text for layout. One implementation is live per tree.
///
/// `metrics` ignores color. A color change keeps the lines and the frozen
/// glyphs. The trait is not `Send`: a typography context stays on the
/// thread that created it.
///
/// [`Shaper::begin_frame`] and [`Shaper::end_frame`] bracket every layout and
/// panel measure in one frame. The tree opens the bracket on the first call
/// and closes it when recording, so a second layout does not drop the cache.
pub trait Shaper {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    /// Measurement-cache hits and misses in the current stats interval.
    fn measure_cache_counts(&self) -> (u32, u32) {
        (0, 0)
    }
    /// Register a face used by later measurements. The default ignores it.
    /// A window shares one font library, so drawing needs no second copy.
    fn register_font(&mut self, _family: &str, _bytes: &[u8]) -> bool {
        false
    }
    fn register_shared_font(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        self.register_font(family, &bytes)
    }

    #[cfg(debug_assertions)]
    fn unregister_dev_font(&mut self, _family: &str) {}

    /// Size, lines, and frozen glyphs at the key's constraint.
    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics>;
    /// Intrinsic width for this geometry.
    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        self.min_content_width(&key.text, f32::from_bits(key.font_size))
    }
    /// Width of the widest unbreakable word. Prefer [`Shaper::intrinsic_width`].
    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        let word = utf8
            .split_whitespace()
            .map(|word| self.measure_word(word, font_size))
            .fold(0.0_f32, f32::max);
        // An empty or whitespace-only text still reserves one cluster so a
        // min-content column does not collapse to zero.
        word.max(self.measure_word(" ", font_size))
    }
    /// Width of one unbounded word at `font_size`. Shared by the
    /// min-content default; not part of the typesetting contract.
    #[doc(hidden)]
    fn measure_word(&mut self, word: &str, font_size: f32) -> f32 {
        let key = MetricsKey {
            text: Arc::from(word),
            style: Arc::new(crate::scene::TextStyle::default()),
            spans: Arc::from([]),
            font_size: font_size.to_bits(),
            max_width: f32::MAX.to_bits(),
        };
        self.metrics(&key).size.width
    }
    /// Intrinsic width and the metrics of the unbounded paragraph.
    fn min_content_metrics(&mut self, key: &MetricsKey) -> (f32, Arc<ParagraphMetrics>) {
        (self.intrinsic_width(key), self.metrics(key))
    }
    /// The node still displays this key. Default does nothing.
    fn touch(&mut self, _key: &MetricsKey) {}
    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret;
    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret;
    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32);
    /// Single-line canvas metrics. The default reports that this shaper has
    /// no font box, so canvas text fails instead of inventing a zero box.
    fn canvas_metrics(
        &mut self,
        _text: &str,
        _style: &crate::scene::TextStyle,
        _font_size: f32,
    ) -> Option<CanvasMetrics> {
        None
    }
    /// Single-line canvas metrics plus frozen glyphs. The default wraps
    /// [`Shaper::canvas_metrics`] with no glyphs, for shapers that only measure.
    fn shape_canvas(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        size: f32,
    ) -> Option<CanvasTextLayout> {
        self.canvas_metrics(text, style, size)
            .map(|metrics| CanvasTextLayout {
                metrics,
                shaped: None,
            })
    }
}

impl Shaper for Box<dyn Shaper> {
    fn begin_frame(&mut self) {
        (**self).begin_frame();
    }
    fn end_frame(&mut self) {
        (**self).end_frame();
    }
    fn measure_cache_counts(&self) -> (u32, u32) {
        (**self).measure_cache_counts()
    }
    fn register_font(&mut self, family: &str, bytes: &[u8]) -> bool {
        (**self).register_font(family, bytes)
    }

    #[cfg(debug_assertions)]
    fn unregister_dev_font(&mut self, family: &str) {
        (**self).unregister_dev_font(family);
    }

    fn register_shared_font(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        (**self).register_shared_font(family, bytes)
    }
    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        (**self).metrics(key)
    }
    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        (**self).intrinsic_width(key)
    }
    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        (**self).min_content_width(utf8, font_size)
    }
    fn min_content_metrics(&mut self, key: &MetricsKey) -> (f32, Arc<ParagraphMetrics>) {
        (**self).min_content_metrics(key)
    }
    fn touch(&mut self, key: &MetricsKey) {
        (**self).touch(key);
    }
    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        (**self).caret_at_unit(key, unit, affinity)
    }
    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        (**self).caret_at_point(key, x, y)
    }
    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32) {
        (**self).word_range(key, unit)
    }
    fn canvas_metrics(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        font_size: f32,
    ) -> Option<CanvasMetrics> {
        (**self).canvas_metrics(text, style, font_size)
    }
    fn shape_canvas(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        size: f32,
    ) -> Option<CanvasTextLayout> {
        (**self).shape_canvas(text, style, size)
    }
}

impl<S: Shaper> Shaper for std::rc::Rc<std::cell::RefCell<S>> {
    fn begin_frame(&mut self) {
        self.borrow_mut().begin_frame();
    }
    fn end_frame(&mut self) {
        self.borrow_mut().end_frame();
    }
    fn measure_cache_counts(&self) -> (u32, u32) {
        self.borrow().measure_cache_counts()
    }
    fn register_font(&mut self, family: &str, bytes: &[u8]) -> bool {
        self.borrow_mut().register_font(family, bytes)
    }

    #[cfg(debug_assertions)]
    fn unregister_dev_font(&mut self, family: &str) {
        self.borrow_mut().unregister_dev_font(family);
    }

    fn register_shared_font(&mut self, family: &str, bytes: Arc<[u8]>) -> bool {
        self.borrow_mut().register_shared_font(family, bytes)
    }
    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        self.borrow_mut().metrics(key)
    }
    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        self.borrow_mut().intrinsic_width(key)
    }
    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        self.borrow_mut().min_content_width(utf8, font_size)
    }
    fn min_content_metrics(&mut self, key: &MetricsKey) -> (f32, Arc<ParagraphMetrics>) {
        self.borrow_mut().min_content_metrics(key)
    }
    fn touch(&mut self, key: &MetricsKey) {
        self.borrow_mut().touch(key);
    }
    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        self.borrow_mut().caret_at_unit(key, unit, affinity)
    }
    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        self.borrow_mut().caret_at_point(key, x, y)
    }
    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32) {
        self.borrow_mut().word_range(key, unit)
    }
    fn canvas_metrics(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        font_size: f32,
    ) -> Option<CanvasMetrics> {
        self.borrow_mut().canvas_metrics(text, style, font_size)
    }
    fn shape_canvas(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        size: f32,
    ) -> Option<CanvasTextLayout> {
        self.borrow_mut().shape_canvas(text, style, size)
    }
}

/// Test shaper. Each Unicode scalar advances `font_size * 0.5`. A line is one
/// font size tall, or `font_size * style.height` when the multiplier is set.
///
/// Hard breaks are `\n`, `\r`, and `\r\n`. Soft wrap keeps at least one cluster
/// on a line and prefers to break after whitespace. This is not a font metric.
/// The host measures with Parley.
#[derive(Debug, Default)]
pub struct MonoShaper {
    /// True between [`Shaper::begin_frame`] and [`Shaper::end_frame`].
    #[allow(
        dead_code,
        reason = "the bracket is observable only through layout timing"
    )]
    open: bool,
}

impl MonoShaper {
    pub fn new() -> Self {
        Self { open: false }
    }

    /// Source text of a paragraph the [`MonoShaper`] froze. Each glyph ID is
    /// one UTF-16 unit of the original text. `None` for any other shaper.
    #[cfg(test)]
    pub fn text_of(shaped: &crate::scene::ShapedParagraph) -> Option<String> {
        let units: Vec<u16> = shaped
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id))
            .collect();
        if shaped.runs.iter().any(|run| run.face.0 != 0) {
            return None;
        }
        String::from_utf16(&units).ok()
    }
}

impl Shaper for MonoShaper {
    fn begin_frame(&mut self) {
        self.open = true;
    }

    fn end_frame(&mut self) {
        self.open = false;
    }

    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        let laid = layout_key(key);
        let baseline = laid.lines.first().map_or(0.0, |line| line.baseline);
        let shaped = (!key.text.is_empty()).then(|| {
            let glyphs: Vec<_> = key
                .text
                .encode_utf16()
                .map(|id| crate::scene::PositionedGlyph {
                    id,
                    x: 0.0,
                    y: baseline,
                })
                .collect();
            Arc::new(crate::scene::ShapedParagraph {
                size: [laid.size.width, laid.size.height],
                runs: Box::new([crate::scene::ShapedRun {
                    face: Arc::new(crate::scene::FontFaceId(0)),
                    font_size: f32::from_bits(key.font_size),
                    normalized_coords: Box::new([]),
                    embolden: false,
                    skew_degrees: None,
                    span_index: None,
                    span_bounds: [0.0, 0.0, laid.size.width, laid.size.height],
                    glyphs: glyphs.into(),
                }]),
                decorations: Box::new([]),
            })
        });
        match shaped {
            Some(shaped) => Arc::new(ParagraphMetrics::shaped(
                laid.size,
                Arc::from(laid.lines),
                baseline,
                shaped,
            )),
            None => Arc::new(ParagraphMetrics::with_baseline(
                laid.size,
                Arc::from(laid.lines),
                baseline,
            )),
        }
    }

    /// Monospace: the widest word is its cluster count times half the font
    /// size, and no metrics lookup is needed.
    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        let widest = utf8
            .split_whitespace()
            .map(|word| crate::text::grapheme_count(word) as f32 * font_size * 0.5)
            .fold(0.0_f32, f32::max);
        widest.max(font_size * 0.5)
    }

    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        let laid = layout_key(key);
        caret_for_unit(&laid, unit, affinity, f32::from_bits(key.font_size))
    }

    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        let laid = layout_key(key);
        caret_at_point(&laid, x, y, f32::from_bits(key.font_size))
    }

    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32) {
        whitespace_word(&key.text, unit)
    }

    fn canvas_metrics(
        &mut self,
        text: &str,
        style: &crate::scene::TextStyle,
        font_size: f32,
    ) -> Option<CanvasMetrics> {
        let _ = style.slant;
        let size = if font_size.is_finite() {
            font_size.max(0.0)
        } else {
            0.0
        };
        let width = text.chars().count() as f32 * size * 0.5;
        Some(CanvasMetrics {
            width,
            ascent: size * 0.8,
            descent: size * 0.2,
        })
    }
}

fn layout_key(key: &MetricsKey) -> MonoLayout {
    layout_mono(
        &key.text,
        f32::from_bits(key.font_size),
        f32::from_bits(key.max_width),
        &key.style,
    )
}

#[derive(Clone, Copy)]
struct Cluster {
    start: u32,
    end: u32,
    advance: f32,
    newline: bool,
    space: bool,
}

struct MonoLayout {
    lines: Vec<TextLine>,
    clusters: Vec<Cluster>,
    size: TextSize,
}

/// Word around `unit`. Whitespace is its own empty range at that offset.
pub fn whitespace_word(text: &str, unit: u32) -> (u32, u32) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let mut index = index_for_unit(&chars, unit).min(chars.len().saturating_sub(1));
    if chars[index].is_whitespace() {
        let at = utf16_len(&chars[..index]);
        return (at, at);
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    let start = index;
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    (utf16_len(&chars[..start]), utf16_len(&chars[..index]))
}

fn layout_mono(
    text: &str,
    font_size: f32,
    max_width: f32,
    style: &crate::scene::TextStyle,
) -> MonoLayout {
    let line_height = line_height(font_size, style);
    if font_size <= 0.0 || max_width <= 0.0 || text.is_empty() {
        return MonoLayout {
            lines: Vec::new(),
            clusters: Vec::new(),
            size: TextSize {
                width: 0.0,
                height: 0.0,
            },
        };
    }
    let clusters = clusters_of(text, font_size);
    let mut lines = wrap_lines(&clusters, max_width, line_height, style);
    apply_ellipsis(&mut lines, &clusters, font_size, max_width, style);
    align_lines(&mut lines, max_width, style);
    for line in &mut lines {
        line.baseline = font_size * 0.8;
    }
    let widest = lines.iter().map(|line| line.w).fold(0.0, f32::max);
    let width = match style.overflow {
        crate::scene::TextOverflow::Visible => widest,
        _ if clusters.len() == 1 && widest > max_width => widest,
        _ => widest.min(max_width),
    };
    let height = line_height * lines.len() as f32;
    MonoLayout {
        lines,
        clusters,
        size: TextSize { width, height },
    }
}

fn line_height(font_size: f32, style: &crate::scene::TextStyle) -> f32 {
    // The mono font's own line box is one em. Parley applies the line height
    // itself, so this multiplier is only for the test shaper.
    resolved_line_height(style, font_size, font_size)
}

fn clusters_of(text: &str, font_size: f32) -> Vec<Cluster> {
    let step = font_size * 0.5;
    let mut clusters = Vec::new();
    let mut chars = text.chars().peekable();
    let mut unit = 0u32;
    while let Some(ch) = chars.next() {
        if ch == '\r' || ch == '\n' {
            let mut end = unit + crate::cast::u32_from_usize(ch.len_utf16());
            if ch == '\r' && chars.peek() == Some(&'\n') {
                let next = chars.next().unwrap_or('\n');
                end += crate::cast::u32_from_usize(next.len_utf16());
            }
            clusters.push(Cluster {
                start: unit,
                end,
                advance: 0.0,
                newline: true,
                space: false,
            });
            unit = end;
            continue;
        }
        let mut end = unit + crate::cast::u32_from_usize(ch.len_utf16());
        let mut advance = step;
        let space = ch.is_whitespace();
        while let Some(next) = chars.peek().copied() {
            if next == '\n' || next == '\r' || !is_mark(next) {
                break;
            }
            chars.next();
            end += crate::cast::u32_from_usize(next.len_utf16());
            advance += step;
        }
        clusters.push(Cluster {
            start: unit,
            end,
            advance,
            newline: false,
            space,
        });
        unit = end;
    }
    clusters
}

fn is_mark(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

fn wrap_lines(
    clusters: &[Cluster],
    max_width: f32,
    line_height: f32,
    style: &crate::scene::TextStyle,
) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut index = 0usize;
    while index < clusters.len() {
        let mut width = 0.0f32;
        let mut last_space: Option<usize> = None;
        let mut cursor = index;
        let mut hard = false;
        while cursor < clusters.len() {
            let cluster = &clusters[cursor];
            if cluster.newline {
                hard = true;
                break;
            }
            let overflow = width + cluster.advance > max_width + f32::EPSILON;
            if style.soft_wrap && overflow && cursor > index {
                break;
            }
            width += cluster.advance;
            if cluster.space {
                last_space = Some(cursor);
            }
            cursor += 1;
        }
        if hard {
            push_line(&mut lines, clusters, index, cursor, line_height, true);
            index = cursor + 1;
            if index == clusters.len() {
                push_line(&mut lines, clusters, index, index, line_height, false);
            }
            continue;
        }
        if style.soft_wrap && cursor < clusters.len() && cursor > index {
            if let Some(space) = last_space.filter(|slot| slot + 1 < cursor) {
                push_line(&mut lines, clusters, index, space + 1, line_height, false);
                index = space + 1;
            } else {
                push_line(&mut lines, clusters, index, cursor, line_height, false);
                index = cursor;
            }
            continue;
        }
        push_line(&mut lines, clusters, index, cursor, line_height, false);
        index = cursor.max(index + 1);
    }
    lines
}

fn push_line(
    lines: &mut Vec<TextLine>,
    clusters: &[Cluster],
    from: usize,
    to: usize,
    line_height: f32,
    hard_break: bool,
) {
    let start = if from < clusters.len() {
        clusters[from].start
    } else {
        clusters.last().map(|cluster| cluster.end).unwrap_or(0)
    };
    let (end, end_with_break, width) = if hard_break {
        let newline = &clusters[to];
        let width = clusters
            .get(from..to)
            .unwrap_or(&[])
            .iter()
            .map(|cluster| cluster.advance)
            .sum();
        (newline.start, newline.end, width)
    } else {
        let content = clusters.get(from..to).unwrap_or(&[]);
        let end = content.last().map(|cluster| cluster.end).unwrap_or(start);
        let width = content.iter().map(|cluster| cluster.advance).sum();
        (end, end, width)
    };
    lines.push(TextLine {
        start,
        end,
        end_with_break,
        x: 0.0,
        y: lines.len() as f32 * line_height,
        w: width,
        h: line_height,
        hard_break,
        baseline: 0.0,
    });
}

fn apply_ellipsis(
    lines: &mut Vec<TextLine>,
    clusters: &[Cluster],
    font_size: f32,
    max_width: f32,
    style: &crate::scene::TextStyle,
) {
    if lines.is_empty() || style.overflow == crate::scene::TextOverflow::Visible {
        return;
    }
    let limit = style
        .max_lines
        .filter(|max| *max > 0)
        .map(|max| max as usize)
        .unwrap_or(lines.len());
    let mut hidden = false;
    if lines.len() > limit {
        lines.truncate(limit);
        hidden = true;
    }
    if style.overflow != crate::scene::TextOverflow::Ellipsis {
        return;
    }
    let Some(mark) = style.ellipsis_str() else {
        return;
    };
    let Some(last) = lines.last_mut() else {
        return;
    };
    if !hidden && last.w <= max_width + f32::EPSILON {
        return;
    }
    let tail = mark.chars().count() as f32 * font_size * 0.5;
    let budget = (max_width - tail).max(0.0);
    let owned: Vec<Cluster> = clusters
        .iter()
        .copied()
        .filter(|cluster| {
            cluster.start >= last.start && cluster.start < last.end && !cluster.newline
        })
        .collect();
    let mut kept = last.w;
    let mut end = last.end;
    for cluster in owned.iter().rev() {
        if kept <= budget + f32::EPSILON {
            break;
        }
        kept -= cluster.advance;
        end = cluster.start;
    }
    last.end = end;
    last.end_with_break = end;
    last.hard_break = false;
    last.w = kept.max(0.0) + tail;
}

fn align_lines(lines: &mut [TextLine], max_width: f32, style: &crate::scene::TextStyle) {
    for line in lines.iter_mut() {
        let extra = (max_width - line.w).max(0.0);
        let left = match style.align {
            crate::scene::TextAlign::Left => 0.0,
            crate::scene::TextAlign::Right => extra,
            crate::scene::TextAlign::Center => extra * 0.5,
        };
        line.x = if style.direction_rtl && style.align != crate::scene::TextAlign::Center {
            extra - left
        } else {
            left
        };
    }
}

fn caret_at_point(laid: &MonoLayout, x: f32, y: f32, font_size: f32) -> Caret {
    if laid.lines.is_empty() {
        return empty_caret(font_size, Affinity::Downstream);
    }
    let index = laid
        .lines
        .iter()
        .position(|line| y < line.y + line.h)
        .unwrap_or(laid.lines.len() - 1);
    let line = &laid.lines[index];
    let mut cursor = line.x;
    let mut unit = line.start;
    for cluster in line_clusters(&laid.clusters, line) {
        let mid = cursor + cluster.advance * 0.5;
        if x < mid {
            return caret_box(unit, cursor, line, Affinity::Downstream);
        }
        cursor += cluster.advance;
        unit = cluster.end;
    }
    caret_box(line.end, cursor, line, Affinity::Downstream)
}

fn caret_for_unit(laid: &MonoLayout, unit: u32, affinity: Affinity, font_size: f32) -> Caret {
    if laid.lines.is_empty() {
        return empty_caret(font_size, affinity);
    }
    let end = laid
        .lines
        .last()
        .map(|line| line.end_with_break)
        .unwrap_or(0);
    let unit = unit.min(end);
    if affinity == Affinity::Upstream
        && let Some((index, _)) = laid
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.start == unit)
        && index > 0
        && !laid.lines[index - 1].hard_break
    {
        let prev = &laid.lines[index - 1];
        return caret_box(unit, prev.x + prev.w, prev, Affinity::Upstream);
    }
    // `lines` is non-empty: the empty layout returned above.
    let Some(line) = laid
        .lines
        .iter()
        .find(|line| unit < line.end_with_break)
        .or(laid.lines.last())
    else {
        return empty_caret(font_size, affinity);
    };
    let mut cursor = line.x;
    for cluster in line_clusters(&laid.clusters, line) {
        if unit <= cluster.start {
            break;
        }
        cursor += cluster.advance;
        if unit < cluster.end {
            break;
        }
    }
    caret_box(unit, cursor, line, affinity)
}

fn line_clusters<'a>(
    clusters: &'a [Cluster],
    line: &TextLine,
) -> impl Iterator<Item = &'a Cluster> {
    clusters.iter().filter(move |cluster| {
        !cluster.newline && cluster.start >= line.start && cluster.start < line.end
    })
}

fn caret_box(unit: u32, x: f32, line: &TextLine, affinity: Affinity) -> Caret {
    Caret {
        unit,
        x,
        y: line.y,
        w: 1.0,
        h: line.h,
        affinity,
    }
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

fn index_for_unit(chars: &[char], unit: u32) -> usize {
    let mut seen = 0u32;
    for (index, ch) in chars.iter().enumerate() {
        if seen >= unit {
            return index;
        }
        seen += u32::try_from(ch.len_utf16()).unwrap_or(1);
    }
    chars.len()
}

fn utf16_len(chars: &[char]) -> u32 {
    chars
        .iter()
        .map(|ch| u32::try_from(ch.len_utf16()).unwrap_or(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(
        text: &str,
        font_size: f32,
        max_width: f32,
        style: crate::scene::TextStyle,
    ) -> MetricsKey {
        metrics_key(Arc::from(text), Arc::new(style), &[], font_size, max_width)
    }

    fn style_ellipsis(max_lines: u32) -> crate::scene::TextStyle {
        crate::scene::TextStyle {
            max_lines: Some(max_lines),
            overflow: crate::scene::TextOverflow::Ellipsis,
            ..crate::scene::TextStyle::default()
        }
    }

    #[test]
    fn hard_break_puts_the_caret_on_the_next_line() {
        let mut shaper = MonoShaper::new();
        let shaped = key("a\nb", 16.0, 100.0, crate::scene::TextStyle::default());
        let metrics = shaper.metrics(&shaped);
        let lines = &metrics.lines;
        assert_eq!(lines.len(), 2);
        assert!(lines[0].hard_break);
        assert_eq!(lines[1].start, 2);
        let caret = shaper.caret_at_unit(&shaped, 2, Affinity::Downstream);
        assert!((caret.y - 16.0).abs() < 0.1, "y {}", caret.y);
        assert!((caret.x - 0.0).abs() < 0.1);
    }

    #[test]
    fn soft_wrap_affinity_distinguishes_the_line_edges() {
        let mut shaper = MonoShaper::new();
        // Advance is 8. Width 20 keeps two scalars on the first line.
        let shaped = key("abcd", 16.0, 20.0, crate::scene::TextStyle::default());
        let metrics = shaper.metrics(&shaped);
        let lines = &metrics.lines;
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].hard_break);
        assert_eq!(lines[1].start, 2);
        let down = shaper.caret_at_unit(&shaped, 2, Affinity::Downstream);
        let up = shaper.caret_at_unit(&shaped, 2, Affinity::Upstream);
        assert!((down.y - 16.0).abs() < 0.1, "down {}", down.y);
        assert!((up.y - 0.0).abs() < 0.1, "up {}", up.y);
        assert!(up.x > down.x);
    }

    #[test]
    fn ellipsis_drops_the_second_line_and_reserves_the_mark() {
        let mut shaper = MonoShaper::new();
        let shaped = key("abcdef", 16.0, 40.0, style_ellipsis(1));
        let size = shaper.metrics(&shaped).size;
        assert!((size.height - 16.0).abs() < 0.1, "h {}", size.height);
        assert!(size.width <= 40.0 + 0.1, "w {}", size.width);
        let metrics = shaper.metrics(&shaped);
        let lines = &metrics.lines;
        assert_eq!(lines.len(), 1);
        assert!(lines[0].end < 6);
    }

    #[test]
    fn a_disabled_soft_wrap_keeps_one_wide_line() {
        let mut shaper = MonoShaper::new();
        let style = crate::scene::TextStyle {
            soft_wrap: false,
            overflow: crate::scene::TextOverflow::Visible,
            ..crate::scene::TextStyle::default()
        };
        let size = shaper.metrics(&key("abcdef", 16.0, 20.0, style)).size;
        assert!((size.width - 48.0).abs() < 0.1, "w {}", size.width);
        assert!((size.height - 16.0).abs() < 0.1, "h {}", size.height);
    }

    #[test]
    fn a_color_free_key_keeps_metrics_and_freezes_glyphs() {
        let mut shaper = MonoShaper::new();
        let metrics = key("Hi", 16.0, 100.0, crate::scene::TextStyle::default());
        let before = shaper.metrics(&metrics);
        let shaped = before
            .shaped
            .as_ref()
            .expect("non-empty text freezes glyphs");
        assert!(!shaped.runs.is_empty());
        assert_eq!(shaped.size[0], before.size.width);
        assert_eq!(before.size, shaper.metrics(&metrics).size);
        let empty = shaper.metrics(&key("", 16.0, 100.0, crate::scene::TextStyle::default()));
        assert!(empty.shaped.is_none());
    }

    #[test]
    fn mono_shaper_reports_no_canvas_glyphs() {
        let mut shaper = MonoShaper::new();
        let style = crate::scene::TextStyle::default();
        let layout = shaper
            .shape_canvas("Hi", &style, 16.0)
            .expect("mono measures canvas text");
        assert_eq!(layout.metrics.width, 16.0);
        assert!(layout.shaped.is_none());
    }
}
