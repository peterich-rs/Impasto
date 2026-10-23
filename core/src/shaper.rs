/// Paragraph size in scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSize {
    pub width: f32,
    pub height: f32,
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
    ) -> TextSize;
}

/// Test shaper. Each byte advances `font_size * 0.5`. A line is `font_size` tall.
///
/// Bytes that do not fit the line move to the next one. At least one byte stays
/// on a line when the advance is wider than `max_width`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MonoShaper;

impl Shaper for MonoShaper {
    fn measure(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        _color: [u8; 4],
        style: &protocol::TextStyle,
    ) -> TextSize {
        if font_size <= 0.0 || max_width <= 0.0 || text.is_empty() {
            return TextSize {
                width: 0.0,
                height: 0.0,
            };
        }
        let advance = font_size * 0.5;
        let content = advance * text.len() as f32;
        if content <= max_width {
            return TextSize {
                width: content,
                height: capped_height(font_size, 1, style.max_lines),
            };
        }
        let per_line = (max_width / advance).floor().max(1.0) as usize;
        let lines = text.len().div_ceil(per_line);
        let longest = per_line.min(text.len());
        TextSize {
            width: advance * longest as f32,
            height: capped_height(font_size, lines, style.max_lines),
        }
    }
}

fn capped_height(font_size: f32, lines: usize, max_lines: Option<u32>) -> f32 {
    let lines = match max_lines {
        Some(max) if max > 0 => lines.min(max as usize),
        _ => lines,
    };
    font_size * lines as f32
}
