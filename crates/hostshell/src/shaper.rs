use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use uicore::{Affinity, Caret, MetricsKey, PaintKey, ParagraphMetrics, Shaper, TextLine, TextSize};

const WORD_CAP: usize = 4096;

/// Measures on the engine typography context. Drawing reuses those paragraphs.
pub struct ImpellerShaper {
    typography: Rc<RefCell<engine::Typography>>,
    spans: Vec<protocol::TextSpan>,
    words: WordCache,
}

impl ImpellerShaper {
    pub fn new(typography: Rc<RefCell<engine::Typography>>) -> Self {
        Self {
            typography,
            spans: Vec::new(),
            words: WordCache::new(),
        }
    }

    fn fill_spans(&mut self, key: &MetricsKey, color: [u8; 4], span_colors: Option<&[[u8; 4]]>) {
        self.spans.clear();
        for (index, span) in key.spans.iter().enumerate() {
            let span_color = span_colors
                .and_then(|colors| colors.get(index))
                .copied()
                .unwrap_or(color);
            self.spans.push(protocol::TextSpan {
                utf8: span.utf8.to_string(),
                color: span_color,
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
        span_colors: Option<&[[u8; 4]]>,
        body: impl FnOnce(&mut engine::Typography, &[protocol::TextSpan]) -> T,
    ) -> T {
        self.fill_spans(key, color, span_colors);
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
            self.words.clear();
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
            Some(key.span_colors.as_ref()),
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
        self.paint_metrics(&key.metrics, key.color, Some(key.span_colors.as_ref()))
    }

    fn touch(&mut self, key: &PaintKey) {
        self.with_spans(
            &key.metrics,
            key.color,
            Some(key.span_colors.as_ref()),
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

    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        let font_bits = font_size.to_bits();
        let widest = utf8
            .split_whitespace()
            .map(|word| self.word_width(word, font_bits))
            .fold(0.0_f32, f32::max);
        widest.max(self.word_width(" ", font_bits))
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
        span_colors: Option<&[[u8; 4]]>,
    ) -> Arc<ParagraphMetrics> {
        self.with_spans(key, color, span_colors, |typography, spans| {
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

    fn word_width(&mut self, word: &str, font_bits: u32) -> f32 {
        let id = WordKey {
            word: Arc::from(word),
            font_bits,
        };
        if let Some(width) = self.words.get(&id) {
            return width;
        }
        let key = MetricsKey {
            text: Arc::clone(&id.word),
            style: Arc::new(protocol::TextStyle::default()),
            spans: Arc::from([]),
            font_size: font_bits,
            max_width: f32::MAX.to_bits(),
        };
        let width = self.metrics(&key).size.width;
        self.words.insert(id, width);
        width
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct WordKey {
    word: Arc<str>,
    font_bits: u32,
}

struct WordCache {
    map: HashMap<WordKey, f32>,
    order: VecDeque<WordKey>,
}

impl WordCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn get(&self, key: &WordKey) -> Option<f32> {
        self.map.get(key).copied()
    }

    fn insert(&mut self, key: WordKey, width: f32) {
        let key = match self.map.entry(key) {
            Entry::Occupied(mut occupied) => {
                occupied.insert(width);
                return;
            }
            Entry::Vacant(vacant) => vacant.into_key(),
        };
        if self.order.len() >= WORD_CAP
            && let Some(old) = self.order.pop_front()
        {
            self.map.remove(&old);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, width);
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
