use std::cell::RefCell;
use std::rc::Rc;

use protocol::{TextSpan, TextStyle};
use ui_core::{Caret, Shaper, TextSize};

/// Measures with the same typography context the player draws with.
pub struct ImpellerShaper {
    engine: Rc<RefCell<engine::Engine>>,
}

impl ImpellerShaper {
    pub fn new(engine: Rc<RefCell<engine::Engine>>) -> Self {
        Self { engine }
    }
}

impl Shaper for ImpellerShaper {
    fn measure(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> TextSize {
        match self
            .engine
            .borrow_mut()
            .measure_paragraph(text, font_size, max_width, color, style)
        {
            Ok((width, height)) => TextSize { width, height },
            Err(_) => TextSize {
                width: 0.0,
                height: 0.0,
            },
        }
    }

    fn measure_spans(
        &self,
        text: &str,
        spans: &[TextSpan],
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
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
        match self
            .engine
            .borrow_mut()
            .measure_with_spans(utf8, font_size, max_width, color, style, spans)
        {
            Ok((width, height)) => TextSize { width, height },
            Err(_) => TextSize {
                width: 0.0,
                height: 0.0,
            },
        }
    }

    fn caret_at_point(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Caret {
        self.engine
            .borrow_mut()
            .caret_at_point(text, font_size, max_width, color, style, x, y)
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn caret_at_unit(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> Caret {
        self.caret_for(
            text,
            font_size,
            max_width,
            color,
            style,
            unit,
            ui_core::Affinity::Downstream,
        )
    }

    fn caret_for(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
        affinity: ui_core::Affinity,
    ) -> Caret {
        self.engine
            .borrow_mut()
            .caret_for(
                text,
                font_size,
                max_width,
                color,
                style,
                unit,
                affinity == ui_core::Affinity::Upstream,
            )
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn lines(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> Vec<ui_core::TextLine> {
        self.engine
            .borrow_mut()
            .lines(text, font_size, max_width, color, style)
            .unwrap_or_default()
            .into_iter()
            .map(|line| ui_core::TextLine {
                start: line.start,
                end: line.end,
                end_with_break: line.end_with_break,
                x: line.x,
                y: line.y,
                w: line.w,
                h: line.h,
                hard_break: line.hard_break,
            })
            .collect()
    }

    fn word_range(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
        unit: u32,
    ) -> (u32, u32) {
        self.engine
            .borrow_mut()
            .word_range(text, font_size, max_width, color, style, unit)
            .unwrap_or((0, 0))
    }
}

fn caret_from_hit(hit: engine::CaretHit) -> Caret {
    Caret {
        unit: hit.unit,
        x: hit.x,
        y: hit.y,
        w: hit.w,
        h: hit.h,
        affinity: if hit.upstream {
            ui_core::Affinity::Upstream
        } else {
            ui_core::Affinity::Downstream
        },
    }
}

fn empty_caret(font_size: f32) -> Caret {
    Caret {
        unit: 0,
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: font_size.max(0.0),
        affinity: ui_core::Affinity::Downstream,
    }
}
