use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use uicore::{Affinity, Caret, MetricsKey, PaintKey, ParagraphMetrics, Shaper, TextLine, TextSize};

const INTRINSIC_CAP: usize = 4096;

/// Measures on the engine typography context. Drawing reuses those paragraphs.
pub struct ImpellerShaper {
    typography: Rc<RefCell<engine::Typography>>,
    spans: Vec<protocol::TextSpan>,
    /// Min-content widths. The key ignores color and layout width.
    intrinsic: HashMap<MetricsKey, f32>,
}

impl ImpellerShaper {
    pub fn new(typography: Rc<RefCell<engine::Typography>>) -> Self {
        Self {
            typography,
            spans: Vec::new(),
            intrinsic: HashMap::new(),
        }
    }

    fn fill_spans(
        &mut self,
        key: &MetricsKey,
        color: [u8; 4],
        span_paints: Option<&[protocol::SpanPaint]>,
    ) {
        self.spans.clear();
        for (index, span) in key.spans.iter().enumerate() {
            let span_color = match span_paints.and_then(|paints| paints.get(index)) {
                Some(protocol::SpanPaint::Color(paint)) => *paint,
                Some(protocol::SpanPaint::Gradient(_)) | None => color,
            };
            self.spans.push(protocol::TextSpan {
                utf8: span.utf8.to_string(),
                color: span_color,
                gradient: None,
                weight: span.weight,
                underline: span.underline,
                line_through: span.line_through,
                family: if span.family.is_empty() {
                    None
                } else {
                    Some(span.family.to_string())
                },
            });
        }
    }

    fn with_spans<T>(
        &mut self,
        key: &MetricsKey,
        color: [u8; 4],
        span_paints: Option<&[protocol::SpanPaint]>,
        body: impl FnOnce(&mut engine::Typography, &[protocol::TextSpan]) -> T,
    ) -> T {
        self.fill_spans(key, color, span_paints);
        let spans = std::mem::take(&mut self.spans);
        let value = {
            let mut typography = self.typography.borrow_mut();
            body(&mut typography, &spans)
        };
        self.spans = spans;
        value
    }
}

impl Shaper for ImpellerShaper {
    fn begin_frame(&mut self) {
        self.typography.borrow_mut().begin_measure();
    }

    fn end_frame(&mut self) {
        self.typography.borrow_mut().end_measure();
    }

    fn register_font(&mut self, family: &str, bytes: &[u8]) -> bool {
        let registered = self
            .typography
            .borrow_mut()
            .register_face(bytes, family)
            .is_ok();
        if registered {
            self.intrinsic.clear();
        }
        registered
    }

    fn metrics(&mut self, key: &MetricsKey) -> Arc<ParagraphMetrics> {
        self.paint_metrics(key, [0, 0, 0, 255], None)
    }

    fn measure_paint(&mut self, key: &PaintKey) -> TextSize {
        self.with_spans(
            &key.metrics,
            key.color,
            Some(key.span_paints.as_ref()),
            |typography, spans| {
                typography
                    .measure_with_spans(
                        &key.metrics.text,
                        f32::from_bits(key.metrics.font_size),
                        f32::from_bits(key.metrics.max_width),
                        key.color,
                        key.metrics.style.as_ref(),
                        spans,
                    )
                    .map(|(width, height)| TextSize { width, height })
                    .unwrap_or(TextSize {
                        width: 0.0,
                        height: 0.0,
                    })
            },
        )
    }

    fn metrics_paint(&mut self, key: &PaintKey) -> Arc<ParagraphMetrics> {
        self.paint_metrics(&key.metrics, key.color, Some(key.span_paints.as_ref()))
    }

    fn touch(&mut self, key: &PaintKey) {
        self.with_spans(
            &key.metrics,
            key.color,
            Some(key.span_paints.as_ref()),
            |typography, spans| {
                typography.touch_measure(
                    &key.metrics.text,
                    f32::from_bits(key.metrics.font_size),
                    f32::from_bits(key.metrics.max_width),
                    key.color,
                    key.metrics.style.as_ref(),
                    spans,
                );
            },
        );
    }

    fn intrinsic_width(&mut self, key: &MetricsKey) -> f32 {
        let id = MetricsKey {
            text: Arc::clone(&key.text),
            style: Arc::clone(&key.style),
            spans: Arc::clone(&key.spans),
            font_size: key.font_size,
            max_width: 0,
        };
        if let Some(width) = self.intrinsic.get(&id) {
            return *width;
        }
        self.fill_spans(&id, [0, 0, 0, 255], None);
        let spans = std::mem::take(&mut self.spans);
        let width = {
            let typography = self.typography.borrow();
            typography
                .min_intrinsic_width(
                    &id.text,
                    f32::from_bits(id.font_size),
                    id.style.as_ref(),
                    &spans,
                )
                .unwrap_or(0.0)
        };
        self.spans = spans;
        if self.intrinsic.len() >= INTRINSIC_CAP {
            self.intrinsic.clear();
        }
        self.intrinsic.insert(id, width);
        width
    }

    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        let key = MetricsKey {
            text: Arc::from(utf8),
            style: Arc::new(protocol::TextStyle::default()),
            spans: Arc::from([]),
            font_size: font_size.to_bits(),
            max_width: 0,
        };
        self.intrinsic_width(&key)
    }

    fn caret_at_unit(&mut self, key: &MetricsKey, unit: u32, affinity: Affinity) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        self.typography
            .borrow_mut()
            .caret_for(
                engine::ParagraphQuery {
                    utf8: &key.text,
                    font_size,
                    max_width: f32::from_bits(key.max_width),
                    color: [0, 0, 0, 255],
                    style: key.style.as_ref(),
                },
                unit,
                affinity == Affinity::Upstream,
            )
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        self.typography
            .borrow_mut()
            .caret_at_point(
                engine::ParagraphQuery {
                    utf8: &key.text,
                    font_size,
                    max_width: f32::from_bits(key.max_width),
                    color: [0, 0, 0, 255],
                    style: key.style.as_ref(),
                },
                x,
                y,
            )
            .map(caret_from_hit)
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn word_range(&mut self, key: &MetricsKey, unit: u32) -> (u32, u32) {
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

impl ImpellerShaper {
    fn paint_metrics(
        &mut self,
        key: &MetricsKey,
        color: [u8; 4],
        span_paints: Option<&[protocol::SpanPaint]>,
    ) -> Arc<ParagraphMetrics> {
        self.with_spans(key, color, span_paints, |typography, spans| {
            let style = key.style.as_ref();
            let font_size = f32::from_bits(key.font_size);
            let max_width = f32::from_bits(key.max_width);
            let size = typography
                .measure_with_spans(&key.text, font_size, max_width, color, style, spans)
                .map(|(width, height)| TextSize { width, height })
                .unwrap_or(TextSize {
                    width: 0.0,
                    height: 0.0,
                });
            let measured = typography
                .lines_with_spans(&key.text, font_size, max_width, color, style, spans)
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
        })
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
