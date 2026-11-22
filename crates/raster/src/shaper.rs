use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use engine::text::{
    Affinity, CanvasMetrics, Caret, MetricsKey, PaintKey, ParagraphMetrics, ParagraphQuery, Shaper,
    TextSize,
};

const INTRINSIC_CAP: usize = 4096;

/// Measures on the engine typography context. Drawing reuses those paragraphs.
pub struct ImpellerShaper {
    typography: Rc<RefCell<crate::Typography>>,
    /// Min-content widths. The key ignores color and layout width.
    intrinsic: HashMap<MetricsKey, f32>,
}

impl ImpellerShaper {
    pub fn new(typography: Rc<RefCell<crate::Typography>>) -> Self {
        Self {
            typography,
            intrinsic: HashMap::new(),
        }
    }
}

impl Shaper for ImpellerShaper {
    fn begin_frame(&mut self) {
        self.typography.borrow_mut().begin_measure();
    }

    fn end_frame(&mut self) {
        self.typography.borrow_mut().end_measure();
    }

    fn measure_cache_counts(&self) -> (u32, u32) {
        let stats = self.typography.borrow().shape_stats();
        (stats.measure_hit, stats.measure_miss)
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
        self.typography
            .borrow_mut()
            .measure_key(&key.metrics, key.color, Some(&key.span_paints))
            .map(|(width, height)| TextSize { width, height })
            .unwrap_or(TextSize {
                width: 0.0,
                height: 0.0,
            })
    }

    fn metrics_paint(&mut self, key: &PaintKey) -> Arc<ParagraphMetrics> {
        self.paint_metrics(&key.metrics, key.color, Some(key.span_paints.as_ref()))
    }

    fn min_content_metrics(&mut self, key: &PaintKey) -> (f32, Arc<ParagraphMetrics>) {
        let metrics = self.metrics_paint(key);
        let width = self
            .typography
            .borrow_mut()
            .measured_min_intrinsic_key(key)
            .unwrap_or(0.0);
        (width, metrics)
    }

    fn touch(&mut self, key: &PaintKey) {
        self.typography.borrow_mut().touch_key(key);
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
        let width = self
            .typography
            .borrow()
            .min_intrinsic_key(&id)
            .unwrap_or(0.0);
        if self.intrinsic.len() >= INTRINSIC_CAP {
            self.intrinsic.clear();
        }
        self.intrinsic.insert(id, width);
        width
    }

    fn min_content_width(&mut self, utf8: &str, font_size: f32) -> f32 {
        let key = MetricsKey {
            text: Arc::from(utf8),
            style: Arc::new(engine::scene::TextStyle::default()),
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
                ParagraphQuery {
                    utf8: &key.text,
                    font_size,
                    max_width: f32::from_bits(key.max_width),
                    color: [0, 0, 0, 255],
                    style: key.style.as_ref(),
                },
                unit,
                affinity == Affinity::Upstream,
            )
            .unwrap_or_else(|_| empty_caret(font_size))
    }

    fn caret_at_point(&mut self, key: &MetricsKey, x: f32, y: f32) -> Caret {
        let font_size = f32::from_bits(key.font_size);
        self.typography
            .borrow_mut()
            .caret_at_point(
                ParagraphQuery {
                    utf8: &key.text,
                    font_size,
                    max_width: f32::from_bits(key.max_width),
                    color: [0, 0, 0, 255],
                    style: key.style.as_ref(),
                },
                x,
                y,
            )
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

    fn canvas_metrics(
        &mut self,
        text: &str,
        style: &engine::scene::TextStyle,
        font_size: f32,
    ) -> Option<CanvasMetrics> {
        if !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        // Same single-line style the player draws, so italic stays on the key
        // and an empty run still has a font box. Do not scale the font size.
        let style = single_line_style(style);
        let probe = if text.is_empty() { " " } else { text };
        let lines = self
            .typography
            .borrow_mut()
            .lines(probe, font_size, 1_000_000.0, [0, 0, 0, 255], &style)
            .ok()?;
        let first = lines.first()?;
        let width = if text.is_empty() {
            0.0
        } else {
            lines
                .iter()
                .fold(0.0_f32, |widest, line| widest.max(line.w))
        };
        Some(CanvasMetrics {
            width,
            ascent: first.baseline,
            descent: first.h - first.baseline,
        })
    }
}

fn single_line_style(style: &engine::scene::TextStyle) -> engine::scene::TextStyle {
    let mut style = style.clone();
    style.soft_wrap = false;
    style.ellipsis = None;
    style.overflow = engine::scene::TextOverflow::Visible;
    style.max_lines = None;
    style.align = engine::scene::TextAlign::Left;
    style
}

impl ImpellerShaper {
    fn paint_metrics(
        &mut self,
        key: &MetricsKey,
        color: [u8; 4],
        span_paints: Option<&[engine::scene::SpanPaint]>,
    ) -> Arc<ParagraphMetrics> {
        let mut typography = self.typography.borrow_mut();
        let (size, lines) = match typography.measure_key(key, color, span_paints) {
            Ok((width, height)) => (
                TextSize { width, height },
                typography
                    .lines_key(key, color, span_paints)
                    .unwrap_or_default(),
            ),
            Err(_) => (
                TextSize {
                    width: 0.0,
                    height: 0.0,
                },
                Vec::new(),
            ),
        };
        let baseline = lines.first().map_or(0.0, |line| line.baseline);
        Arc::new(ParagraphMetrics::with_baseline(
            size,
            lines.into(),
            baseline,
        ))
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

#[cfg(test)]
mod canvas_metrics_tests {
    use super::ImpellerShaper;
    use engine::text::Shaper;

    #[test]
    fn empty_text_keeps_the_font_box_and_italic_measures() {
        let engine = match crate::Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(crate::Status::NotLinked) => return,
            Err(status) => panic!("create_metal: {status}"),
        };
        let Ok(font) = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf") else {
            return;
        };
        let mut engine = engine;
        if engine.register_font(&font, "Sans").is_err() {
            return;
        }
        let mut shaper = ImpellerShaper::new(engine.typography());
        let style = engine::scene::TextStyle::default();
        let Some(empty) = shaper.canvas_metrics("", &style, 16.0) else {
            panic!("empty metrics");
        };
        let Some(letter) = shaper.canvas_metrics("M", &style, 16.0) else {
            panic!("letter metrics");
        };
        assert_eq!(empty.width, 0.0);
        assert!(empty.ascent > 0.0);
        assert!(letter.width > 0.0);
        assert_eq!(empty.ascent, letter.ascent);
        assert_eq!(empty.descent, letter.descent);
        let italic = engine::scene::TextStyle {
            slant: engine::scene::FontSlant::Italic,
            ..engine::scene::TextStyle::default()
        };
        let Some(slanted) = shaper.canvas_metrics("M", &italic, 16.0) else {
            panic!("italic metrics");
        };
        assert!(slanted.width > 0.0);
        assert!(shaper.canvas_metrics("M", &style, 0.0).is_none());
        assert!(shaper.canvas_metrics("M", &style, f32::NAN).is_none());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn real_quote_layout_matches_a_full_pass_for_digits_signs_and_cjk() -> Result<(), engine::Error>
    {
        let mut engine = match crate::Rasterizer::create_metal() {
            Ok(engine) => engine,
            Err(crate::Status::NotLinked) => return Ok(()),
            Err(status) => panic!("create_metal: {status}"),
        };
        let latin = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("Arial");
        let cjk = std::fs::read("/System/Library/Fonts/Hiragino Sans GB.ttc").expect("CJK font");
        engine
            .register_font(&latin, "Sans")
            .expect("register Arial");
        engine.register_font(&cjk, "SansCJK").expect("register CJK");

        let fixture = || -> Result<_, engine::Error> {
            let mut tree = engine::tree::Tree::new(ImpellerShaper::new(engine.typography()));
            tree.set_viewport(0.0, 0.0, 122.0, 250.0)?;
            let root = tree.row(0.0)?;
            tree.set_size(root, 122.0, 220.0)?;
            tree.set_row_cross(root, engine::tree::RowCrossAlign::Baseline)?;
            let price = tree.text("43.12", 16.0, [255; 4])?;
            let label = tree.text("港股 12.4", 12.0, [255; 4])?;
            tree.set_max_axis(
                label,
                engine::tree::AxisSize::Points(46.0),
                engine::tree::AxisSize::Auto,
            )?;
            tree.add_child(root, price)?;
            tree.add_child(root, label)?;
            tree.layout()?;
            tree.record()?;
            Ok((tree, price, label))
        };
        let (mut optimized, price, label) = fixture()?;
        let (mut reference, ref_price, ref_label) = fixture()?;
        for (value, name) in [
            ("35.24", "港股 23.5"),
            ("-35.24", "港股报价 23.5"),
            ("-3.5", "港股报价−100.00"),
        ] {
            optimized.set_text(price, value)?;
            optimized.set_text(label, name)?;
            reference.set_text(ref_price, value)?;
            reference.set_text(ref_label, name)?;
            reference.set_viewport(0.0, 0.0, 122.0, 250.0)?;
            optimized.layout()?;
            reference.layout()?;
            assert_eq!(optimized.rect(price)?, reference.rect(ref_price)?);
            assert_eq!(optimized.rect(label)?, reference.rect(ref_label)?);
            assert_eq!(optimized.record()?, reference.record()?);
        }
        Ok(())
    }
}
