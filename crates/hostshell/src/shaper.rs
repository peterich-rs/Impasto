use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use uicore::{Affinity, Caret, MetricsKey, PaintKey, ParagraphMetrics, Shaper, TextLine, TextSize};

/// Measures with the same typography context the player draws with.
///
/// The borrow must end before `session.frame()` returns. `submit_layers` borrows
/// this same `Typography` only after that.
pub struct ImpellerShaper {
    typography: Rc<RefCell<engine::Typography>>,
}

impl ImpellerShaper {
    pub fn new(typography: Rc<RefCell<engine::Typography>>) -> Self {
        Self { typography }
    }
}

impl Shaper for ImpellerShaper {
    fn metrics(&self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        let style = key.style.as_ref();
        let font_size = f32::from_bits(key.font_size);
        let max_width = f32::from_bits(key.max_width);
        let spans = spans_from(key);
        let mut engine = self.typography.borrow_mut();
        let size = engine
            .measure_with_spans(
                &key.text,
                font_size,
                max_width,
                [0, 0, 0, 255],
                style,
                &spans,
            )
            .map(|(width, height)| TextSize { width, height })
            .unwrap_or(TextSize {
                width: 0.0,
                height: 0.0,
            });
        let measured = engine
            .lines_with_spans(
                &key.text,
                font_size,
                max_width,
                [0, 0, 0, 255],
                style,
                &spans,
            )
            .unwrap_or_default();
        let baseline = measured.first().map(|line| line.baseline).unwrap_or(0.0);
        let lines = measured
            .into_iter()
            .map(|line| TextLine {
                start: line.start,
                end: line.end,
                end_with_break: line.end_with_break,
                x: line.x,
                y: line.y,
                w: line.w,
                h: line.h,
                hard_break: line.hard_break,
            })
            .collect::<Vec<_>>();
        Arc::new(ParagraphMetrics::with_baseline(
            size,
            Arc::from(lines),
            baseline,
        ))
    }

    fn measure_paint(&self, key: &PaintKey) -> TextSize {
        let spans = paint_spans(key);
        let style = key.metrics.style.as_ref();
        self.typography
            .borrow_mut()
            .measure_paint(
                &key.metrics.text,
                f32::from_bits(key.metrics.font_size),
                f32::from_bits(key.metrics.max_width),
                key.color,
                style,
                &spans,
            )
            .map(|(width, height)| TextSize { width, height })
            .unwrap_or(TextSize {
                width: 0.0,
                height: 0.0,
            })
    }

    fn retain_measured(&self, key: &PaintKey) -> (protocol::ParagraphId, Arc<ParagraphMetrics>) {
        let spans = paint_spans(key);
        let style = key.metrics.style.as_ref();
        let measured = self.typography.borrow_mut().retain_measured(
            &key.metrics.text,
            f32::from_bits(key.metrics.font_size),
            f32::from_bits(key.metrics.max_width),
            key.color,
            style,
            &spans,
        );
        match measured {
            Ok((id, width, height, lines, baseline)) => {
                let lines = lines
                    .into_iter()
                    .map(|line| TextLine {
                        start: line.start,
                        end: line.end,
                        end_with_break: line.end_with_break,
                        x: line.x,
                        y: line.y,
                        w: line.w,
                        h: line.h,
                        hard_break: line.hard_break,
                    })
                    .collect::<Vec<_>>();
                (
                    id,
                    Arc::new(ParagraphMetrics::with_baseline(
                        TextSize { width, height },
                        Arc::from(lines),
                        baseline,
                    )),
                )
            }
            Err(_) => (protocol::ParagraphId::NONE, self.metrics(&key.metrics)),
        }
    }

    fn retain_paint(&self, key: &PaintKey) -> protocol::ParagraphId {
        let spans = paint_spans(key);
        self.typography
            .borrow_mut()
            .retain_paragraph(
                &key.metrics.text,
                f32::from_bits(key.metrics.font_size),
                f32::from_bits(key.metrics.max_width),
                key.color,
                key.metrics.style.as_ref(),
                &spans,
            )
            .unwrap_or(protocol::ParagraphId::NONE)
    }

    fn release_paint(&self, id: protocol::ParagraphId) {
        self.typography.borrow_mut().release_paragraph(id);
    }

    fn caret_at_unit(&self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        self.typography
            .borrow_mut()
            .caret_for(
                &key.text,
                font_size,
                f32::from_bits(key.max_width),
                [0, 0, 0, 255],
                key.style.as_ref(),
                unit,
                affinity == Affinity::Upstream,
            )
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn caret_at_point(&self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        self.typography
            .borrow_mut()
            .caret_at_point(
                &key.text,
                font_size,
                f32::from_bits(key.max_width),
                [0, 0, 0, 255],
                key.style.as_ref(),
                x,
                y,
            )
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn word_range(&self, key: &MetricsKey, unit: u32) -> (u32, u32) {
        self.typography
            .borrow_mut()
            .word_range(
                &key.text,
                f32::from_bits(key.font_size),
                f32::from_bits(key.max_width),
                [0, 0, 0, 255],
                key.style.as_ref(),
                unit,
            )
            .unwrap_or((0, 0))
    }
}

fn paint_spans(key: &PaintKey) -> Vec<protocol::TextSpan> {
    key.metrics
        .spans
        .iter()
        .enumerate()
        .map(|(index, span)| protocol::TextSpan {
            utf8: span.utf8.to_string(),
            color: key.span_colors.get(index).copied().unwrap_or(key.color),
            weight: span.weight,
            underline: span.underline,
            line_through: span.line_through,
            family: if span.family.is_empty() {
                None
            } else {
                Some(span.family.to_string())
            },
        })
        .collect()
}

fn spans_from(key: &MetricsKey) -> Vec<protocol::TextSpan> {
    key.spans
        .iter()
        .map(|span| protocol::TextSpan {
            utf8: span.utf8.to_string(),
            color: [0, 0, 0, 255],
            weight: span.weight,
            underline: span.underline,
            line_through: span.line_through,
            family: if span.family.is_empty() {
                None
            } else {
                Some(span.family.to_string())
            },
        })
        .collect()
}

fn caret_from_hit(hit: engine::CaretHit) -> Caret {
    Caret {
        unit: hit.unit,
        x: hit.x,
        y: hit.y,
        w: hit.w,
        h: hit.h,
        affinity: if hit.upstream {
            Affinity::Upstream
        } else {
            Affinity::Downstream
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
        affinity: Affinity::Downstream,
    }
}
