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
}

/// Line limit used when the parent does not set a width.
///
/// Measurement and the later text command both use this value, so a single
/// line is shaped the same way when it is drawn.
pub const SINGLE_LINE_MAX: f32 = 1_000_000.0;

/// Measures a paragraph. One implementation is live per tree.
///
/// `max_width` is the paragraph constraint in scene points. A wide value keeps
/// the text on one line. The returned width is the longest line, at most
/// `max_width`. `color` is the foreground baked into the shaped run when the
/// shaper keeps that run for drawing. `style` is the same value later copied
/// onto the text command.
pub trait Shaper {
    fn measure(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> TextSize {
        self.measure_spans(text, &[], font_size, max_width, color, style)
    }

    /// Measure `text` with optional styled runs. An empty `spans` slice is the
    /// command's single style. A non-empty slice is the same string the player
    /// draws; implementors must not drop span family, weight, or color.
    fn measure_spans(
        &self,
        text: &str,
        spans: &[protocol::TextSpan],
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> TextSize;

    /// Caret nearest `(x, y)` in paragraph space. The point is relative to the
    /// paragraph origin, the same space [`Shaper::measure`] uses.
    fn caret_at_point(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
        x: f32,
        y: f32,
    ) -> Caret;

    /// Caret rectangle for a UTF-16 index. Indexes past the end clamp to the end.
    ///
    /// The caret uses [`Affinity::Downstream`].
    fn caret_at_unit(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
        unit: u32,
    ) -> Caret;

    /// Caret at `unit`, using `affinity` when that offset is a soft line break.
    fn caret_for(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
        unit: u32,
        affinity: Affinity,
    ) -> Caret {
        let _ = affinity;
        self.caret_at_unit(text, font_size, max_width, color, style, unit)
    }

    /// Lines of the paragraph, in paint order. Empty when the text is empty.
    fn lines(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> Vec<TextLine> {
        let _ = (text, font_size, max_width, color, style);
        Vec::new()
    }

    /// UTF-16 range of the word containing `unit`. Whitespace separates words.
    fn word_range(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
        unit: u32,
    ) -> (u32, u32);
}

/// Test shaper. Each Unicode scalar advances `font_size * 0.5`. A line is one
/// font size tall, or `font_size * style.height` when the multiplier is set.
///
/// Hard breaks are `\n`, `\r`, and `\r\n`. Soft wrap keeps at least one cluster
/// on a line and prefers to break after whitespace. This is not a font metric.
/// The host measures with Impeller.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonoShaper;

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

impl Shaper for MonoShaper {
    fn measure_spans(
        &self,
        text: &str,
        spans: &[protocol::TextSpan],
        font_size: f32,
        max_width: f32,
        _color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> TextSize {
        let owned;
        let utf8 = if spans.is_empty() {
            text
        } else {
            owned = spans
                .iter()
                .map(|span| span.utf8.as_str())
                .collect::<String>();
            debug_assert_eq!(owned, text);
            &owned
        };
        layout_mono(utf8, font_size, max_width, style).size
    }

    fn caret_at_point(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        _color: [u8; 4],
        style: &protocol::TextStyle,
        x: f32,
        y: f32,
    ) -> Caret {
        let laid = layout_mono(text, font_size, max_width, style);
        caret_at_point(&laid, x, y, font_size)
    }

    fn caret_at_unit(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &protocol::TextStyle,
        unit: u32,
    ) -> Caret {
        self.caret_for(
            text,
            font_size,
            max_width,
            color,
            style,
            unit,
            Affinity::Downstream,
        )
    }

    fn caret_for(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        _color: [u8; 4],
        style: &protocol::TextStyle,
        unit: u32,
        affinity: Affinity,
    ) -> Caret {
        let laid = layout_mono(text, font_size, max_width, style);
        caret_for_unit(&laid, unit, affinity, font_size)
    }

    fn lines(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        _color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> Vec<TextLine> {
        layout_mono(text, font_size, max_width, style).lines
    }

    fn word_range(
        &self,
        text: &str,
        _font_size: f32,
        _max_width: f32,
        _color: [u8; 4],
        _style: &protocol::TextStyle,
        unit: u32,
    ) -> (u32, u32) {
        whitespace_word(text, unit)
    }
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
    style: &protocol::TextStyle,
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
    let widest = lines.iter().map(|line| line.w).fold(0.0, f32::max);
    let width = match style.overflow {
        protocol::TextOverflow::Visible => widest,
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

fn line_height(font_size: f32, style: &protocol::TextStyle) -> f32 {
    if style.height.is_finite() && style.height > 0.0 {
        font_size * style.height
    } else {
        font_size
    }
}

fn clusters_of(text: &str, font_size: f32) -> Vec<Cluster> {
    let step = font_size * 0.5;
    let mut clusters = Vec::new();
    let mut chars = text.chars().peekable();
    let mut unit = 0u32;
    while let Some(ch) = chars.next() {
        if ch == '\r' || ch == '\n' {
            let mut end = unit + ch.len_utf16() as u32;
            if ch == '\r' && chars.peek() == Some(&'\n') {
                let next = chars.next().unwrap_or('\n');
                end += next.len_utf16() as u32;
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
        let mut end = unit + ch.len_utf16() as u32;
        let mut advance = step;
        let space = ch.is_whitespace();
        while let Some(next) = chars.peek().copied() {
            if next == '\n' || next == '\r' || !is_mark(next) {
                break;
            }
            chars.next();
            end += next.len_utf16() as u32;
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
    style: &protocol::TextStyle,
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
    });
}

fn apply_ellipsis(
    lines: &mut Vec<TextLine>,
    clusters: &[Cluster],
    font_size: f32,
    max_width: f32,
    style: &protocol::TextStyle,
) {
    if lines.is_empty() || style.overflow == protocol::TextOverflow::Visible {
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
    if style.overflow != protocol::TextOverflow::Ellipsis {
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

fn align_lines(lines: &mut [TextLine], max_width: f32, style: &protocol::TextStyle) {
    for line in lines.iter_mut() {
        let extra = (max_width - line.w).max(0.0);
        let left = match style.align {
            protocol::TextAlign::Left => 0.0,
            protocol::TextAlign::Right => extra,
            protocol::TextAlign::Center => extra * 0.5,
        };
        line.x = if style.direction_rtl && style.align != protocol::TextAlign::Center {
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
    let line = laid
        .lines
        .iter()
        .find(|line| unit < line.end_with_break)
        .unwrap_or(laid.lines.last().unwrap());
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

    fn style_ellipsis(max_lines: u32) -> protocol::TextStyle {
        protocol::TextStyle {
            max_lines: Some(max_lines),
            overflow: protocol::TextOverflow::Ellipsis,
            ..protocol::TextStyle::default()
        }
    }

    #[test]
    fn hard_break_puts_the_caret_on_the_next_line() {
        let shaper = MonoShaper;
        let style = protocol::TextStyle::default();
        let lines = shaper.lines("a\nb", 16.0, 100.0, [0, 0, 0, 255], &style);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].hard_break);
        assert_eq!(lines[1].start, 2);
        let caret = shaper.caret_at_unit("a\nb", 16.0, 100.0, [0, 0, 0, 255], &style, 2);
        assert!((caret.y - 16.0).abs() < 0.1, "y {}", caret.y);
        assert!((caret.x - 0.0).abs() < 0.1);
    }

    #[test]
    fn soft_wrap_affinity_distinguishes_the_line_edges() {
        let shaper = MonoShaper;
        let style = protocol::TextStyle::default();
        // Advance is 8. Width 20 keeps two scalars on the first line.
        let lines = shaper.lines("abcd", 16.0, 20.0, [0, 0, 0, 255], &style);
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].hard_break);
        assert_eq!(lines[1].start, 2);
        let down = shaper.caret_for(
            "abcd",
            16.0,
            20.0,
            [0, 0, 0, 255],
            &style,
            2,
            Affinity::Downstream,
        );
        let up = shaper.caret_for(
            "abcd",
            16.0,
            20.0,
            [0, 0, 0, 255],
            &style,
            2,
            Affinity::Upstream,
        );
        assert!((down.y - 16.0).abs() < 0.1, "down {}", down.y);
        assert!((up.y - 0.0).abs() < 0.1, "up {}", up.y);
        assert!(up.x > down.x);
    }

    #[test]
    fn ellipsis_drops_the_second_line_and_reserves_the_mark() {
        let shaper = MonoShaper;
        let style = style_ellipsis(1);
        let size = shaper.measure("abcdef", 16.0, 40.0, [0, 0, 0, 255], &style);
        assert!((size.height - 16.0).abs() < 0.1, "h {}", size.height);
        assert!(size.width <= 40.0 + 0.1, "w {}", size.width);
        let lines = shaper.lines("abcdef", 16.0, 40.0, [0, 0, 0, 255], &style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].end < 6);
    }

    #[test]
    fn a_disabled_soft_wrap_keeps_one_wide_line() {
        let shaper = MonoShaper;
        let style = protocol::TextStyle {
            soft_wrap: false,
            overflow: protocol::TextOverflow::Visible,
            ..protocol::TextStyle::default()
        };
        let size = shaper.measure("abcdef", 16.0, 20.0, [0, 0, 0, 255], &style);
        assert!((size.width - 48.0).abs() < 0.1, "w {}", size.width);
        assert!((size.height - 16.0).abs() < 0.1, "h {}", size.height);
    }
}
