//! One draw callback's recorder. The current path is stored in canvas space.

use std::mem::size_of;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::scene::CanvasComposite;
use kurbo::{Affine, BezPath, PathEl, Point, Rect, RoundedRect, RoundedRectRadii, Shape};

use super::paint::{Gradient, PaintSource, alpha_byte, parse_color};
use super::path::{
    self, FillRule, Path2D, contains_fill, contains_stroke, corner_radii, curve_segments,
    extend_curve, extend_line, extend_quad, fit_radii, normalize_rect, transform_path,
};
use super::text::{CanvasFont, TextMetrics};
use super::{
    Budget, CURVE_PIXELS, CanvasError, MAX_DASH, MAX_OPS, MAX_RECORD_BYTES, MAX_SAVES,
    MAX_SEGMENTS, finite,
};

/// Content box in scene points. Zero draws nothing at the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSize {
    pub width: f64,
    pub height: f64,
}

/// Values the draw callback reads from the frame, not from signals inside itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintEnv {
    pub pixel_ratio: f64,
    pub direction_rtl: bool,
}

impl Default for PaintEnv {
    fn default() -> Self {
        Self {
            pixel_ratio: 1.0,
            direction_rtl: false,
        }
    }
}

/// Monotonic deadline shared by the paint turn and native geometry.
#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    end: Instant,
}

impl Deadline {
    pub fn after(duration: Duration) -> Self {
        Self {
            end: Instant::now()
                .checked_add(duration)
                .unwrap_or_else(Instant::now),
        }
    }

    pub fn check(&self) -> Result<(), CanvasError> {
        if Instant::now() >= self.end {
            Err(CanvasError::interrupted())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextBaseline {
    Alphabetic,
    Ideographic,
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDirection {
    Inherit,
    Ltr,
    Rtl,
}

/// Paint fields a context getter reads back. Strings stay on the language side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthorState {
    pub fill_color: Option<[u8; 4]>,
    pub stroke_color: Option<[u8; 4]>,
    pub fill_gradient: bool,
    pub stroke_gradient: bool,
    pub line_width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub alpha: f64,
    pub composite: CanvasComposite,
    pub dash_offset: f64,
    pub align: TextAlign,
    pub baseline: TextBaseline,
    pub direction: TextDirection,
    pub smoothing: bool,
}

/// Authoring paint. A gradient stays live until a drawing call snapshots it.
#[derive(Clone, Debug)]
enum LivePaint {
    Color([u8; 4]),
    Gradient(Gradient),
}

impl LivePaint {
    fn bake(&self) -> PaintSource {
        match self {
            Self::Color(color) => PaintSource::Color(*color),
            Self::Gradient(gradient) => PaintSource::Gradient(gradient.snapshot()),
        }
    }
}

fn color_of(source: &LivePaint) -> Option<[u8; 4]> {
    match source {
        LivePaint::Color(color) => Some(*color),
        LivePaint::Gradient(_) => None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StrokeSnap {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dash: Rc<[f64]>,
    pub dash_offset: f64,
}

#[derive(Clone, Debug)]
struct DrawState {
    fill: LivePaint,
    stroke: LivePaint,
    line_width: f64,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
    alpha: f64,
    composite: CanvasComposite,
    dash: Rc<[f64]>,
    dash_offset: f64,
    font: CanvasFont,
    align: TextAlign,
    baseline: TextBaseline,
    direction: TextDirection,
    smoothing: bool,
    ctm: Affine,
    inherited_rtl: bool,
}

impl DrawState {
    fn new(inherited_rtl: bool) -> Self {
        Self {
            fill: LivePaint::Color([0, 0, 0, 255]),
            stroke: LivePaint::Color([0, 0, 0, 255]),
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            alpha: 1.0,
            composite: CanvasComposite::SourceOver,
            dash: Rc::from([]),
            dash_offset: 0.0,
            font: CanvasFont::default(),
            align: TextAlign::Start,
            baseline: TextBaseline::Alphabetic,
            direction: TextDirection::Inherit,
            smoothing: true,
            ctm: Affine::IDENTITY,
            inherited_rtl,
        }
    }

    fn rtl(&self) -> bool {
        match self.direction {
            TextDirection::Inherit => self.inherited_rtl,
            TextDirection::Ltr => false,
            TextDirection::Rtl => true,
        }
    }

    fn stroke_snap(&self) -> StrokeSnap {
        StrokeSnap {
            width: self.line_width,
            cap: self.cap,
            join: self.join,
            miter_limit: self.miter_limit,
            dash: Rc::clone(&self.dash),
            dash_offset: self.dash_offset,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RecOp {
    Save,
    Restore,
    Clip {
        path: BezPath,
        fill: FillRule,
    },
    Fill {
        path: BezPath,
        asset_space: bool,
        fill: FillRule,
        paint: PaintSource,
        alpha: f64,
        composite: CanvasComposite,
        ctm: Affine,
    },
    SolidRect {
        rect: Rect,
        radius: f64,
        color: [u8; 4],
        alpha: f64,
    },
    Stroke {
        path: BezPath,
        asset_space: bool,
        style: StrokeSnap,
        paint: PaintSource,
        alpha: f64,
        composite: CanvasComposite,
        ctm: Affine,
    },
    Clear {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        ctm: Affine,
    },
    Text {
        text: String,
        x: f64,
        y: f64,
        max_width: Option<f64>,
        align: TextAlign,
        baseline: TextBaseline,
        direction_rtl: bool,
        font: CanvasFont,
        paint: PaintSource,
        alpha: f64,
        composite: CanvasComposite,
        ctm: Affine,
    },
    Image {
        source: String,
        src: Option<[f64; 4]>,
        x: f64,
        y: f64,
        w: Option<f64>,
        h: Option<f64>,
        smoothing: bool,
        alpha: f64,
        composite: CanvasComposite,
        ctm: Affine,
    },
}

/// Owned recording. Dropping it refunds the temporary budget charge.
pub struct Recording<'a> {
    pub(crate) ops: Vec<RecOp>,
    pub(crate) pixel_ratio: f64,
    pub(crate) budget: &'a Budget,
    charged: usize,
}

impl Drop for Recording<'_> {
    fn drop(&mut self) {
        self.budget.refund(self.charged);
    }
}

impl Recording<'_> {
    pub(crate) fn release_charge(&mut self) {
        let charged = self.charged;
        self.charged = 0;
        self.budget.refund(charged);
    }
}

/// Records one draw. The path survives `save` / `restore`.
pub struct Recorder<'a> {
    path: BezPath,
    /// A single primitive stays valid until a new path segment is recorded.
    fill_primitive: Option<(usize, Rect, f64)>,
    state: DrawState,
    stack: Vec<DrawState>,
    ops: Vec<RecOp>,
    segments: usize,
    size: CanvasSize,
    env: PaintEnv,
    budget: &'a Budget,
    charged: usize,
}

impl Drop for Recorder<'_> {
    fn drop(&mut self) {
        self.budget.refund(self.charged);
    }
}

impl<'a> Recorder<'a> {
    pub fn new(size: CanvasSize, env: PaintEnv, budget: &'a Budget) -> Self {
        Self {
            path: BezPath::new(),
            fill_primitive: None,
            state: DrawState::new(env.direction_rtl),
            stack: Vec::new(),
            ops: Vec::new(),
            segments: 0,
            size,
            env,
            budget,
            charged: 0,
        }
    }

    pub fn size(&self) -> CanvasSize {
        self.size
    }

    pub fn width(&self) -> f64 {
        self.size.width
    }

    pub fn height(&self) -> f64 {
        self.size.height
    }

    pub fn finish(mut self) -> Recording<'a> {
        let leftover = self.stack.len();
        for _ in 0..leftover {
            self.ops.push(RecOp::Restore);
        }
        self.stack.clear();
        let charged = self.charged;
        self.charged = 0;
        Recording {
            ops: std::mem::take(&mut self.ops),
            pixel_ratio: self.env.pixel_ratio,
            budget: self.budget,
            charged,
        }
    }

    pub fn save(&mut self) -> Result<(), CanvasError> {
        if self.stack.len() >= MAX_SAVES {
            return Err(CanvasError::limit("save"));
        }
        self.charge(64)?;
        self.stack.push(self.state.clone());
        self.push_only(RecOp::Save)
    }

    pub fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
            self.ops.push(RecOp::Restore);
        }
    }

    pub fn begin_path(&mut self) {
        self.path.truncate(0);
        self.fill_primitive = None;
    }

    pub fn close_path(&mut self) -> Result<(), CanvasError> {
        let Some(start) = path::closing_start(&self.path) else {
            return Ok(());
        };
        self.add_segments(2)?;
        path::close_subpath(&mut self.path, start);
        Ok(())
    }

    pub fn move_to(&mut self, x: f64, y: f64) -> Result<(), CanvasError> {
        let Some(point) = self.map_point(x, y) else {
            return Ok(());
        };
        self.add_segments(1)?;
        self.path.move_to(point);
        Ok(())
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> Result<(), CanvasError> {
        let Some(point) = self.map_point(x, y) else {
            return Ok(());
        };
        self.add_segments(1)?;
        extend_line(&mut self.path, point);
        Ok(())
    }

    /// Starts one new subpath using consecutive x/y pairs.
    pub fn polyline(&mut self, points: &[f64]) -> Result<(), CanvasError> {
        self.append_points(points, 2, "polyline")
    }

    /// Starts an independent subpath for each x0/y0/x1/y1 group.
    pub fn line_segments(&mut self, points: &[f64]) -> Result<(), CanvasError> {
        self.append_points(points, 4, "lineSegments")
    }

    fn append_points(
        &mut self,
        points: &[f64],
        group: usize,
        api: &'static str,
    ) -> Result<(), CanvasError> {
        if !points.len().is_multiple_of(group) {
            return Err(CanvasError::argument(api, "incomplete coordinate group"));
        }
        let count = points.len() / 2;
        if self.segments.saturating_add(count) > MAX_SEGMENTS {
            return Err(CanvasError::limit(api));
        }
        let (pairs, _) = points.as_chunks::<2>();
        let map = |pair: &[f64; 2]| self.state.ctm * Point::new(pair[0], pair[1]);
        if !pairs.iter().all(|pair| {
            let mapped = map(pair);
            pair.iter().all(|value| finite(*value)) && finite(mapped.x) && finite(mapped.y)
        }) {
            return Err(CanvasError::argument(api, "non-finite point"));
        }
        let ctm = self.state.ctm;
        self.add_segments(count)?;
        for (index, pair) in pairs.iter().enumerate() {
            let point = ctm * Point::new(pair[0], pair[1]);
            if index == 0 || (group == 4 && index % 2 == 0) {
                self.path.move_to(point);
            } else {
                extend_line(&mut self.path, point);
            }
        }
        Ok(())
    }

    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) -> Result<(), CanvasError> {
        let (Some(control), Some(point)) = (self.map_point(cx, cy), self.map_point(x, y)) else {
            return Ok(());
        };
        let count = curve_segments(&self.path);
        self.add_segments(count)?;
        extend_quad(&mut self.path, control, point);
        Ok(())
    }

    pub fn cubic_to(
        &mut self,
        c1x: f64,
        c1y: f64,
        c2x: f64,
        c2y: f64,
        x: f64,
        y: f64,
    ) -> Result<(), CanvasError> {
        let (Some(c1), Some(c2), Some(point)) = (
            self.map_point(c1x, c1y),
            self.map_point(c2x, c2y),
            self.map_point(x, y),
        ) else {
            return Ok(());
        };
        let count = curve_segments(&self.path);
        self.add_segments(count)?;
        extend_curve(&mut self.path, c1, c2, point);
        Ok(())
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), CanvasError> {
        let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
            return Ok(());
        };
        let rect = Rect::new(x, y, x + w, y + h);
        self.append_primitive(&rect.to_path(0.1), false, rect, 0.0)
    }

    pub fn round_rect(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radii: &[f64],
    ) -> Result<(), CanvasError> {
        for radius in radii {
            if *radius < 0.0 {
                return Err(CanvasError::argument("roundRect", "negative radius"));
            }
        }
        let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
            return Ok(());
        };
        let fitted = fit_radii(w, h, corner_radii(radii));
        let rect = RoundedRect::from_rect(
            Rect::new(x, y, x + w, y + h),
            RoundedRectRadii::new(fitted[0], fitted[1], fitted[2], fitted[3]),
        );
        let path = rect.to_path(self.tolerance());
        if fitted.iter().all(|radius| *radius == fitted[0]) {
            self.append_primitive(&path, false, rect.rect(), fitted[0])
        } else {
            self.append_user(&path, false)
        }
    }

    pub fn arc(
        &mut self,
        cx: f64,
        cy: f64,
        radius: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> Result<(), CanvasError> {
        if radius < 0.0 {
            return Err(CanvasError::argument("arc", "negative radius"));
        }
        if ![cx, cy, radius, start, end]
            .iter()
            .all(|value| finite(*value))
        {
            return Ok(());
        }
        let sweep = path::arc_sweep(start, end, ccw);
        let arc = kurbo::Arc::new((cx, cy), (radius, radius), start, sweep, 0.0);
        let path = arc.to_path(self.tolerance());
        if radius > 0.0 && sweep.abs() == std::f64::consts::TAU {
            let rect = Rect::new(cx - radius, cy - radius, cx + radius, cy + radius);
            self.append_primitive(&path, true, rect, radius)
        } else {
            self.append_user(&path, true)
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "ellipse takes the same arguments as CanvasRenderingContext2D.ellipse"
    )]
    pub fn ellipse(
        &mut self,
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> Result<(), CanvasError> {
        if rx < 0.0 || ry < 0.0 {
            return Err(CanvasError::argument("ellipse", "negative radius"));
        }
        if ![cx, cy, rx, ry, rotation, start, end]
            .iter()
            .all(|value| finite(*value))
        {
            return Ok(());
        }
        let sweep = path::arc_sweep(start, end, ccw);
        let arc = kurbo::Arc::new((cx, cy), (rx, ry), start, sweep, rotation);
        self.append_user(&arc.to_path(self.tolerance()), true)
    }

    pub fn arc_to(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        radius: f64,
    ) -> Result<(), CanvasError> {
        let mut path = Path2D::new();
        if let Some(current) = self.user_current() {
            path.move_to(current.x, current.y)?;
        }
        path.arc_to(x1, y1, x2, y2, radius)?;
        if self.user_current().is_some() {
            let mut elements: Vec<PathEl> = path.path().elements().to_vec();
            if let Some(PathEl::MoveTo(_)) = elements.first() {
                elements.remove(0);
            }
            let mut trimmed = BezPath::new();
            for element in elements {
                trimmed.push(element);
            }
            self.append_user(&trimmed, false)
        } else {
            self.append_user(path.path(), false)
        }
    }

    pub fn translate(&mut self, x: f64, y: f64) {
        if finite(x) && finite(y) {
            self.state.ctm *= Affine::translate((x, y));
        }
    }

    pub fn rotate(&mut self, radians: f64) {
        if finite(radians) {
            self.state.ctm *= Affine::rotate(radians);
        }
    }

    pub fn scale(&mut self, x: f64, y: f64) {
        if finite(x) && finite(y) {
            self.state.ctm *= Affine::scale_non_uniform(x, y);
        }
    }

    pub fn transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].iter().all(|value| finite(*value)) {
            self.state.ctm *= Affine::new([a, b, c, d, e, f]);
        }
    }

    pub fn set_transform(&mut self, a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) {
        if [a, b, c, d, e, f].iter().all(|value| finite(*value)) {
            self.state.ctm = Affine::new([a, b, c, d, e, f]);
        }
    }

    pub fn reset_transform(&mut self) {
        self.state.ctm = Affine::IDENTITY;
    }

    pub fn get_transform(&self) -> [f64; 6] {
        self.state.ctm.as_coeffs()
    }

    pub fn set_fill_style(&mut self, style: &str) {
        if let Some(color) = parse_color(style) {
            self.state.fill = LivePaint::Color(color);
        }
    }

    pub fn set_stroke_style(&mut self, style: &str) {
        if let Some(color) = parse_color(style) {
            self.state.stroke = LivePaint::Color(color);
        }
    }

    pub fn set_fill_gradient(&mut self, gradient: &Gradient) {
        self.state.fill = LivePaint::Gradient(gradient.clone());
    }

    pub fn set_stroke_gradient(&mut self, gradient: &Gradient) {
        self.state.stroke = LivePaint::Gradient(gradient.clone());
    }

    pub fn fill_color(&self) -> Option<[u8; 4]> {
        color_of(&self.state.fill)
    }

    pub fn stroke_color(&self) -> Option<[u8; 4]> {
        color_of(&self.state.stroke)
    }

    /// Parsed paint state. Author color and font strings stay with the caller.
    pub fn author(&self) -> AuthorState {
        AuthorState {
            fill_color: color_of(&self.state.fill),
            stroke_color: color_of(&self.state.stroke),
            fill_gradient: matches!(self.state.fill, LivePaint::Gradient(_)),
            stroke_gradient: matches!(self.state.stroke, LivePaint::Gradient(_)),
            line_width: self.state.line_width,
            cap: self.state.cap,
            join: self.state.join,
            miter_limit: self.state.miter_limit,
            alpha: self.state.alpha,
            composite: self.state.composite,
            dash_offset: self.state.dash_offset,
            align: self.state.align,
            baseline: self.state.baseline,
            direction: self.state.direction,
            smoothing: self.state.smoothing,
        }
    }

    pub fn set_line_width(&mut self, width: f64) {
        if finite(width) && width > 0.0 {
            self.state.line_width = width;
        }
    }

    pub fn line_width(&self) -> f64 {
        self.state.line_width
    }

    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.state.cap = cap;
    }

    pub fn set_line_join(&mut self, join: LineJoin) {
        self.state.join = join;
    }

    pub fn set_miter_limit(&mut self, limit: f64) {
        if finite(limit) && limit > 0.0 {
            self.state.miter_limit = limit;
        }
    }

    pub fn set_global_alpha(&mut self, alpha: f64) {
        if finite(alpha) && (0.0..=1.0).contains(&alpha) {
            self.state.alpha = alpha;
        }
    }

    pub fn global_alpha(&self) -> f64 {
        self.state.alpha
    }

    pub fn set_composite(&mut self, composite: CanvasComposite) {
        self.state.composite = composite;
    }

    pub fn set_line_dash(&mut self, segments: &[f64]) -> Result<(), CanvasError> {
        let stored = if segments.len() % 2 == 1 {
            segments.len().saturating_mul(2)
        } else {
            segments.len()
        };
        if stored > MAX_DASH {
            return Err(CanvasError::limit("setLineDash"));
        }
        if segments.iter().any(|value| !finite(*value) || *value < 0.0) {
            return Ok(());
        }
        if segments.iter().all(|value| *value == 0.0) {
            self.state.dash = Rc::from([]);
            return Ok(());
        }
        // Saved states and recorded strokes can retain earlier patterns until the
        // recording is released. Each assignment reserves its own storage once.
        self.charge(stored * size_of::<f64>())?;
        self.state.dash = segments.iter().copied().cycle().take(stored).collect();
        Ok(())
    }

    pub fn line_dash(&self) -> &[f64] {
        &self.state.dash
    }

    pub fn set_line_dash_offset(&mut self, offset: f64) {
        if finite(offset) {
            self.state.dash_offset = offset;
        }
    }

    pub fn set_font(&mut self, font: &str) {
        if let Some(parsed) = CanvasFont::parse(font) {
            self.state.font = parsed;
        }
    }

    pub fn font(&self) -> &CanvasFont {
        &self.state.font
    }

    pub fn set_text_align(&mut self, align: TextAlign) {
        self.state.align = align;
    }

    pub fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.state.baseline = baseline;
    }

    pub fn set_direction(&mut self, direction: TextDirection) {
        self.state.direction = direction;
    }

    /// Resolved direction. `inherit` uses the direction captured for this draw.
    pub fn text_rtl(&self) -> bool {
        self.state.rtl()
    }

    pub fn set_image_smoothing(&mut self, enabled: bool) {
        self.state.smoothing = enabled;
    }

    pub fn fill(&mut self, rule: FillRule) -> Result<(), CanvasError> {
        if self.path.is_empty() {
            return Ok(());
        }
        let bytes = self.path.elements().len().saturating_mul(32);
        if let Some((segments, rect, radius)) = self.fill_primitive
            && segments == self.segments
            && let Some(op) = self.solid_rect(rect, radius, &self.state.fill)
        {
            self.charge(bytes)?;
            return self.push_only(op);
        }
        self.charge(bytes)?;
        let path = self.path.clone();
        self.push_only(RecOp::Fill {
            path,
            asset_space: false,
            fill: rule,
            paint: self.state.fill.bake(),
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        })
    }

    pub fn fill_path(&mut self, path: &Path2D, rule: FillRule) -> Result<(), CanvasError> {
        if path.path().is_empty() {
            return Ok(());
        }
        self.charge(path.segments().saturating_mul(32))?;
        let asset_space = path.svg_asset()
            && self.state.composite == CanvasComposite::SourceOver
            && matches!(self.state.fill, LivePaint::Color(_));
        let path = if asset_space {
            path.path().clone()
        } else {
            transform_path(&self.state.ctm, path.path())
        };
        self.push_only(RecOp::Fill {
            path,
            asset_space,
            fill: rule,
            paint: self.state.fill.bake(),
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        })
    }

    pub fn stroke(&mut self) -> Result<(), CanvasError> {
        if let Some(op) = self.stroke_line_rect(self.path.elements()) {
            self.charge(64)?;
            return self.push_only(op);
        }
        self.record_stroke(self.path.clone(), false)
    }

    pub fn stroke_path(&mut self, path: &Path2D) -> Result<(), CanvasError> {
        let asset_space = path.svg_asset()
            && self.state.dash.is_empty()
            && self.state.composite == CanvasComposite::SourceOver
            && matches!(self.state.stroke, LivePaint::Color(_));
        let path = if asset_space {
            path.path().clone()
        } else {
            transform_path(&self.state.ctm, path.path())
        };
        self.record_stroke(path, asset_space)
    }

    pub fn clip(&mut self, rule: FillRule) -> Result<(), CanvasError> {
        let bytes = self.path.elements().len().saturating_mul(32).max(32);
        self.charge(bytes)?;
        self.push_only(RecOp::Clip {
            path: self.path.clone(),
            fill: rule,
        })
    }

    pub fn clip_path(&mut self, path: &Path2D, rule: FillRule) -> Result<(), CanvasError> {
        let bytes = path.segments().saturating_mul(32).max(32);
        self.charge(bytes)?;
        self.push_only(RecOp::Clip {
            path: transform_path(&self.state.ctm, path.path()),
            fill: rule,
        })
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), CanvasError> {
        let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
            return Ok(());
        };
        self.charge(64)?;
        let rect = Rect::new(x, y, x + w, y + h);
        self.push_only(self.fill_rect_op(rect, &self.state.fill.bake()))
    }

    /// Fills the rectangles in input order with the current paint.
    pub fn fill_rects(&mut self, rects: &[f64]) -> Result<(), CanvasError> {
        const API: &str = "fillRects";
        if !rects.len().is_multiple_of(4) {
            return Err(CanvasError::argument(API, "incomplete rectangle"));
        }
        if rects.len() / 4 > MAX_OPS {
            return Err(CanvasError::limit(API));
        }
        if !rects.iter().all(|value| finite(*value)) {
            return Err(CanvasError::argument(API, "non-finite rectangle"));
        }
        let (paths, _) = rects.as_chunks::<4>();
        let count = paths
            .iter()
            .filter(|rect| rect[2] != 0.0 && rect[3] != 0.0)
            .count();
        if self.ops.len().saturating_add(count) > MAX_OPS {
            return Err(CanvasError::limit(API));
        }
        self.charge(count * 64)?;
        let paint = self.state.fill.bake();
        for rect in paths {
            if let Some((x, y, w, h)) = normalize_rect(rect[0], rect[1], rect[2], rect[3]) {
                let rect = Rect::new(x, y, x + w, y + h);
                self.ops.push(self.fill_rect_op(rect, &paint));
            }
        }
        Ok(())
    }

    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), CanvasError> {
        let Some(path) = self.user_rect(x, y, w, h) else {
            return Ok(());
        };
        self.record_stroke(transform_path(&self.state.ctm, &path), false)
    }

    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), CanvasError> {
        let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
            return Ok(());
        };
        self.charge(32)?;
        self.push_only(RecOp::Clear {
            x,
            y,
            w,
            h,
            ctm: self.state.ctm,
        })
    }

    pub fn fill_text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        max_width: Option<f64>,
    ) -> Result<(), CanvasError> {
        if text.is_empty() || !finite(x) || !finite(y) {
            return Ok(());
        }
        if max_width.is_some_and(|width| !finite(width)) {
            return Ok(());
        }
        self.charge(text.len())?;
        self.push_only(RecOp::Text {
            text: text.to_string(),
            x,
            y,
            max_width,
            align: self.state.align,
            baseline: self.state.baseline,
            direction_rtl: self.state.rtl(),
            font: self.state.font.clone(),
            paint: self.state.fill.bake(),
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        })
    }

    pub fn measure_text(
        &self,
        text: &str,
        shaper: &mut dyn crate::text::Shaper,
    ) -> Result<TextMetrics, CanvasError> {
        super::text::measure_font(text, &self.state.font, self.state.rtl(), shaper)
    }

    pub fn draw_image(
        &mut self,
        source: &str,
        src: Option<[f64; 4]>,
        x: f64,
        y: f64,
        w: Option<f64>,
        h: Option<f64>,
    ) -> Result<(), CanvasError> {
        if source.is_empty() {
            return Err(CanvasError::argument("drawImage", "empty source"));
        }
        if !finite(x) || !finite(y) {
            return Ok(());
        }
        let (w, h) = match (w, h) {
            (Some(w), Some(h)) => {
                let Some((nx, ny, nw, nh)) = normalize_rect(x, y, w, h) else {
                    return Ok(());
                };
                return self.push_image(source, src, nx, ny, Some(nw), Some(nh));
            }
            (None, None) => (None, None),
            _ => {
                return Err(CanvasError::argument(
                    "drawImage",
                    "width and height are a pair",
                ));
            }
        };
        if let Some(rect) = src
            && normalize_rect(rect[0], rect[1], rect[2], rect[3]).is_none()
        {
            return Ok(());
        }
        self.push_image(source, src, x, y, w, h)
    }

    pub fn is_point_in_path(&self, x: f64, y: f64, rule: FillRule) -> bool {
        self.point_in_device(&self.path, x, y, rule)
    }

    pub fn is_point_in_path_of(&self, path: &Path2D, x: f64, y: f64, rule: FillRule) -> bool {
        let device = transform_path(&self.state.ctm, path.path());
        self.point_in_device(&device, x, y, rule)
    }

    pub fn is_point_in_stroke(
        &self,
        x: f64,
        y: f64,
        deadline: &Deadline,
    ) -> Result<bool, CanvasError> {
        self.point_in_stroke_device(&self.path, x, y, deadline)
    }

    pub fn is_point_in_stroke_of(
        &self,
        path: &Path2D,
        x: f64,
        y: f64,
        deadline: &Deadline,
    ) -> Result<bool, CanvasError> {
        let device = transform_path(&self.state.ctm, path.path());
        self.point_in_stroke_device(&device, x, y, deadline)
    }

    fn push_image(
        &mut self,
        source: &str,
        src: Option<[f64; 4]>,
        x: f64,
        y: f64,
        w: Option<f64>,
        h: Option<f64>,
    ) -> Result<(), CanvasError> {
        let src = if let Some(rect) = src {
            let Some((sx, sy, sw, sh)) = normalize_rect(rect[0], rect[1], rect[2], rect[3]) else {
                return Ok(());
            };
            Some([sx, sy, sw, sh])
        } else {
            None
        };
        self.charge(source.len().saturating_add(32))?;
        self.push_only(RecOp::Image {
            source: source.to_string(),
            src,
            x,
            y,
            w,
            h,
            smoothing: self.state.smoothing,
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        })
    }

    fn fill_rect_op(&self, rect: Rect, paint: &PaintSource) -> RecOp {
        if let Some((rect, radius)) = map_primitive(self.state.ctm, rect, 0.0)
            && let Some(op) = self.solid_rect(rect, radius, &self.state.fill)
        {
            return op;
        }
        RecOp::Fill {
            path: transform_path(&self.state.ctm, &rect.to_path(0.1)),
            asset_space: false,
            fill: FillRule::NonZero,
            paint: paint.clone(),
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        }
    }

    fn solid_rect(&self, rect: Rect, radius: f64, paint: &LivePaint) -> Option<RecOp> {
        let LivePaint::Color(color) = paint else {
            return None;
        };
        (self.state.composite == CanvasComposite::SourceOver).then_some(RecOp::SolidRect {
            rect,
            radius,
            color: *color,
            alpha: self.state.alpha,
        })
    }

    fn stroke_line_rect(&self, elements: &[PathEl]) -> Option<RecOp> {
        let [PathEl::MoveTo(start), PathEl::LineTo(end)] = elements else {
            return None;
        };
        let [a, b, c, d, ..] = self.state.ctm.as_coeffs();
        if b != 0.0
            || c != 0.0
            || a == 0.0
            || d == 0.0
            || path::invertible(self.state.ctm).is_none()
            || !self.state.dash.is_empty()
            || self.state.cap != LineCap::Butt
        {
            return None;
        }
        let width = self.state.line_width * 0.5;
        let rect = if start.x == end.x && start.y != end.y {
            let half = width * a.abs();
            Rect::from_points((start.x - half, start.y), (start.x + half, end.y))
        } else if start.y == end.y && start.x != end.x {
            let half = width * d.abs();
            Rect::from_points((start.x, start.y - half), (end.x, start.y + half))
        } else {
            return None;
        };
        self.solid_rect(rect, 0.0, &self.state.stroke)
    }

    fn record_stroke(&mut self, path: BezPath, asset_space: bool) -> Result<(), CanvasError> {
        if path.is_empty() {
            return Ok(());
        }
        self.charge(path.elements().len().saturating_mul(32))?;
        self.push_only(RecOp::Stroke {
            path,
            asset_space,
            style: self.state.stroke_snap(),
            paint: self.state.stroke.bake(),
            alpha: self.state.alpha,
            composite: self.state.composite,
            ctm: self.state.ctm,
        })
    }

    fn point_in_device(&self, device: &BezPath, x: f64, y: f64, rule: FillRule) -> bool {
        if !finite(x) || !finite(y) {
            return false;
        }
        contains_fill(device, self.state.ctm * Point::new(x, y), rule)
    }

    fn point_in_stroke_device(
        &self,
        device: &BezPath,
        x: f64,
        y: f64,
        deadline: &Deadline,
    ) -> Result<bool, CanvasError> {
        if !finite(x) || !finite(y) {
            return Ok(false);
        }
        contains_stroke(
            device,
            self.state.ctm,
            &self.state.stroke_snap(),
            self.state.ctm * Point::new(x, y),
            self.tolerance(),
            deadline,
        )
    }

    fn has_current(&self) -> bool {
        !self.path.elements().is_empty()
    }

    fn user_rect(&self, x: f64, y: f64, w: f64, h: f64) -> Option<BezPath> {
        let (x, y, w, h) = normalize_rect(x, y, w, h)?;
        Some(Rect::new(x, y, x + w, y + h).to_path(0.1))
    }

    fn append_primitive(
        &mut self,
        path: &BezPath,
        connect: bool,
        rect: Rect,
        radius: f64,
    ) -> Result<(), CanvasError> {
        let empty = self.path.is_empty();
        self.append_user(path, connect)?;
        if empty {
            self.fill_primitive = map_primitive(self.state.ctm, rect, radius)
                .map(|(rect, radius)| (self.segments, rect, radius));
        }
        Ok(())
    }

    fn append_user(&mut self, path: &BezPath, connect: bool) -> Result<(), CanvasError> {
        let mut connect = connect && self.has_current();
        let device = transform_path(&self.state.ctm, path);
        for element in device.elements() {
            let element = if connect {
                connect = false;
                match *element {
                    PathEl::MoveTo(point) => PathEl::LineTo(point),
                    other => other,
                }
            } else {
                *element
            };
            self.add_segments(1)?;
            self.path.push(element);
        }
        Ok(())
    }

    fn user_current(&self) -> Option<Point> {
        let point = path::current_point(&self.path)?;
        path::invertible(self.state.ctm).map(|inverse| inverse * point)
    }

    fn map_point(&self, x: f64, y: f64) -> Option<Point> {
        if !finite(x) || !finite(y) {
            return None;
        }
        Some(self.state.ctm * Point::new(x, y))
    }

    fn tolerance(&self) -> f64 {
        let ratio = if self.env.pixel_ratio.is_finite() && self.env.pixel_ratio > 0.0 {
            self.env.pixel_ratio
        } else {
            1.0
        };
        (CURVE_PIXELS / ratio).max(1.0e-4)
    }

    fn add_segments(&mut self, count: usize) -> Result<(), CanvasError> {
        if self.segments.saturating_add(count) > MAX_SEGMENTS {
            return Err(CanvasError::limit("path"));
        }
        self.segments = self.segments.saturating_add(count);
        Ok(())
    }

    fn charge(&mut self, bytes: usize) -> Result<(), CanvasError> {
        let next = self.charged.saturating_add(bytes);
        if next > MAX_RECORD_BYTES {
            return Err(CanvasError::limit("record"));
        }
        self.budget.reserve(bytes)?;
        self.charged = next;
        Ok(())
    }

    fn push_only(&mut self, op: RecOp) -> Result<(), CanvasError> {
        if self.ops.len() >= MAX_OPS {
            return Err(CanvasError::limit("record"));
        }
        self.ops.push(op);
        Ok(())
    }
}

fn map_primitive(ctm: Affine, rect: Rect, radius: f64) -> Option<(Rect, f64)> {
    let [a, b, c, d, ..] = ctm.as_coeffs();
    if b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 || (radius > 0.0 && a.abs() != d.abs()) {
        return None;
    }
    let mapped = Rect::from_points(
        ctm * Point::new(rect.x0, rect.y0),
        ctm * Point::new(rect.x1, rect.y1),
    );
    Some((mapped, radius * a.abs()))
}

pub(crate) fn kurbo_stroke(style: &StrokeSnap) -> kurbo::Stroke {
    kurbo::Stroke::new(style.width)
        .with_caps(match style.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        })
        .with_join(match style.join {
            LineJoin::Miter => kurbo::Join::Miter,
            LineJoin::Round => kurbo::Join::Round,
            LineJoin::Bevel => kurbo::Join::Bevel,
        })
        .with_miter_limit(style.miter_limit)
}

pub(crate) fn alpha_of(alpha: f64) -> u8 {
    alpha_byte(alpha)
}
