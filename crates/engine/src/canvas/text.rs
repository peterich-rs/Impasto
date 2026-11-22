//! Canvas font shorthand and the metrics `measureText` returns.

use crate::scene::{FontSlant, TextOverflow, TextStyle};

use super::finite;

/// Parsed `font` shorthand. Size is scene points.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasFont {
    pub slant: FontSlant,
    pub weight: u16,
    pub size: f64,
    pub families: Vec<String>,
}

impl Default for CanvasFont {
    fn default() -> Self {
        Self {
            slant: FontSlant::Normal,
            weight: 400,
            size: 10.0,
            families: vec!["Sans".to_string()],
        }
    }
}

impl CanvasFont {
    pub fn parse(input: &str) -> Option<Self> {
        parse_font(input)
    }

    pub(crate) fn text_style(&self, direction_rtl: bool) -> TextStyle {
        TextStyle {
            family: self.families.first().cloned(),
            max_lines: Some(1),
            ellipsis: None,
            align: crate::scene::TextAlign::Left,
            weight: self.weight,
            direction_rtl,
            height: 0.0,
            underline: false,
            line_through: false,
            soft_wrap: false,
            overflow: TextOverflow::Visible,
            slant: self.slant,
        }
    }
}

/// Width and font box from one `measureText` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    pub width: f64,
    pub font_bounding_box_ascent: f64,
    pub font_bounding_box_descent: f64,
}

/// Single-line metrics. An empty string reports width 0 and the font box of `" "`.
pub fn measure_font(
    text: &str,
    font: &CanvasFont,
    direction_rtl: bool,
    shaper: &mut dyn crate::text::Shaper,
) -> Result<TextMetrics, super::CanvasError> {
    let flat = flatten_text(text);
    let style = font.text_style(direction_rtl);
    let size = super::narrow(font.size).unwrap_or(0.0);
    let probe = if flat.is_empty() { " " } else { flat.as_str() };
    let metrics = shaper
        .canvas_metrics(probe, &style, size)
        .ok_or_else(|| super::paint::text_unavailable("measureText"))?;
    Ok(TextMetrics {
        width: if flat.is_empty() {
            0.0
        } else {
            f64::from(metrics.width)
        },
        font_bounding_box_ascent: f64::from(metrics.ascent),
        font_bounding_box_descent: f64::from(metrics.descent),
    })
}

pub(crate) fn parse_font(input: &str) -> Option<CanvasFont> {
    let tokens = font_tokens(input);
    let size_at = tokens.iter().position(|token| token.ends_with("px"))?;
    let size_token = tokens[size_at].trim_end_matches("px");
    let size: f64 = size_token.parse().ok()?;
    if !finite(size) || size <= 0.0 {
        return None;
    }
    if size_at + 1 >= tokens.len() {
        return None;
    }
    let mut slant = FontSlant::Normal;
    let mut weight = 400_u16;
    let mut saw_style = false;
    let mut saw_weight = false;
    for token in &tokens[..size_at] {
        match token.as_str() {
            "italic" if !saw_style => {
                slant = FontSlant::Italic;
                saw_style = true;
            }
            "normal" if !saw_style => {
                saw_style = true;
            }
            "normal" if !saw_weight => {
                weight = 400;
                saw_weight = true;
            }
            "bold" if !saw_weight => {
                weight = 700;
                saw_weight = true;
            }
            other if !saw_weight => {
                let parsed: u16 = other.parse().ok()?;
                if !(1..=1000).contains(&parsed) {
                    return None;
                }
                weight = parsed;
                saw_weight = true;
            }
            _ => return None,
        }
    }
    let family_src = tokens[size_at + 1..].join(" ");
    let families = split_families(&family_src);
    if families.is_empty() {
        return None;
    }
    Some(CanvasFont {
        slant,
        weight,
        size,
        families,
    })
}

fn font_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted: Option<char> = None;
    for ch in input.chars() {
        match quoted {
            Some(quote) if ch == quote => {
                quoted = None;
            }
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => {
                quoted = Some(ch);
            }
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn split_families(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|family| {
            family
                .trim()
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .trim()
        })
        .filter(|family| !family.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Canvas fillText is one line. Newlines become a single space.
pub(crate) fn flatten_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push(' ');
        } else if ch == '\n' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}
