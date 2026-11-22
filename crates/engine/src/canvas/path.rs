//! Path construction, stroking, and hit tests. Coordinates stay in f64 until prepare.

use kurbo::{
    Affine, BezPath, PathEl, PathSeg, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2,
};

use super::record::{StrokeSnap, kurbo_stroke};
use super::{CanvasError, Deadline, MAX_RECORD_BYTES, MAX_SEGMENTS, finite};

const MAX_STROKE_VERBS: usize =
    if MAX_SEGMENTS < MAX_RECORD_BYTES / size_of::<crate::scene::PathVerb>() {
        MAX_SEGMENTS
    } else {
        MAX_RECORD_BYTES / size_of::<crate::scene::PathVerb>()
    };

/// A reusable path in its own coordinate space.
#[derive(Clone, Debug)]
pub struct Path2D {
    path: BezPath,
    generation: u64,
    segments: usize,
}

impl Default for Path2D {
    fn default() -> Self {
        Self::new()
    }
}

impl Path2D {
    pub fn new() -> Self {
        Self {
            path: BezPath::new(),
            generation: 1,
            segments: 0,
        }
    }

    pub fn from_path(other: &Self) -> Self {
        Self {
            path: other.path.clone(),
            generation: 1,
            segments: other.segments,
        }
    }

    pub fn from_svg(data: &str) -> Self {
        let path = match BezPath::from_svg(data) {
            Ok(path) => path,
            Err(_) => BezPath::new(),
        };
        let segments = path.elements().len();
        Self {
            path,
            generation: 1,
            segments,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn segments(&self) -> usize {
        self.segments
    }

    pub(crate) fn path(&self) -> &BezPath {
        &self.path
    }

    pub fn clear(&mut self) {
        self.path.truncate(0);
        self.segments = 0;
        self.bump();
    }

    pub fn add_matrix(&mut self, other: &Self, matrix: [f64; 6]) -> Result<(), CanvasError> {
        self.add_path(other, Affine::new(matrix))
    }

    pub fn add_path(&mut self, other: &Self, transform: Affine) -> Result<(), CanvasError> {
        self.push_elements(
            transform_path(&transform, other.path())
                .elements()
                .iter()
                .copied(),
        )
    }

    pub fn close(&mut self) -> Result<(), CanvasError> {
        let Some(start) = closing_start(&self.path) else {
            return Ok(());
        };
        self.reserve_segments(2)?;
        close_subpath(&mut self.path, start);
        self.segments += 2;
        self.bump();
        Ok(())
    }

    pub fn move_to(&mut self, x: f64, y: f64) -> Result<(), CanvasError> {
        self.point_op("moveTo", x, y, |path, point| path.move_to(point))
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> Result<(), CanvasError> {
        if !finite(x) || !finite(y) {
            return Ok(());
        }
        self.reserve_segments(1)?;
        extend_line(&mut self.path, Point::new(x, y));
        self.segments = self.segments.saturating_add(1);
        self.bump();
        Ok(())
    }

    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) -> Result<(), CanvasError> {
        if ![cx, cy, x, y].iter().all(|value| finite(*value)) {
            return Ok(());
        }
        let count = curve_segments(&self.path);
        self.reserve_segments(count)?;
        extend_quad(&mut self.path, Point::new(cx, cy), Point::new(x, y));
        self.segments = self.segments.saturating_add(count);
        self.bump();
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
        if ![c1x, c1y, c2x, c2y, x, y]
            .iter()
            .all(|value| finite(*value))
        {
            return Ok(());
        }
        let count = curve_segments(&self.path);
        self.reserve_segments(count)?;
        extend_curve(
            &mut self.path,
            Point::new(c1x, c1y),
            Point::new(c2x, c2y),
            Point::new(x, y),
        );
        self.segments = self.segments.saturating_add(count);
        self.bump();
        Ok(())
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<(), CanvasError> {
        let Some((x, y, w, h)) = normalize_rect(x, y, w, h) else {
            return Ok(());
        };
        self.push_elements(Rect::new(x, y, x + w, y + h).path_elements(0.1))
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
        self.push_elements(rect.to_path(0.25).elements().iter().copied())
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
        let sweep = arc_sweep(start, end, ccw);
        let arc = kurbo::Arc::new((cx, cy), (radius, radius), start, sweep, 0.0);
        self.push_elements(connect_arc(self.current_point(), &arc.to_path(0.25)))
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
        let sweep = arc_sweep(start, end, ccw);
        let arc = kurbo::Arc::new((cx, cy), (rx, ry), start, sweep, rotation);
        self.push_elements(connect_arc(self.current_point(), &arc.to_path(0.25)))
    }

    pub fn arc_to(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        radius: f64,
    ) -> Result<(), CanvasError> {
        if radius < 0.0 {
            return Err(CanvasError::argument("arcTo", "negative radius"));
        }
        if ![x1, y1, x2, y2, radius].iter().all(|value| finite(*value)) {
            return Ok(());
        }
        let Some(current) = self.current_point() else {
            return self.move_to(x1, y1);
        };
        let (path, next) = tangent_arc(current, Point::new(x1, y1), Point::new(x2, y2), radius);
        self.push_elements(path.elements().iter().copied())?;
        let _ = next;
        Ok(())
    }

    pub(crate) fn current_point(&self) -> Option<Point> {
        current_point(&self.path)
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn reserve_segments(&self, count: usize) -> Result<(), CanvasError> {
        if self.segments.saturating_add(count) > MAX_SEGMENTS {
            Err(CanvasError::limit("path"))
        } else {
            Ok(())
        }
    }

    fn point_op(
        &mut self,
        api: &'static str,
        x: f64,
        y: f64,
        write: impl FnOnce(&mut BezPath, Point),
    ) -> Result<(), CanvasError> {
        if !finite(x) || !finite(y) {
            return Ok(());
        }
        let _ = api;
        if self.segments >= MAX_SEGMENTS {
            return Err(CanvasError::limit("path"));
        }
        write(&mut self.path, Point::new(x, y));
        self.segments = self.segments.saturating_add(1);
        self.bump();
        Ok(())
    }

    fn push_elements(
        &mut self,
        elements: impl IntoIterator<Item = PathEl>,
    ) -> Result<(), CanvasError> {
        for element in elements {
            if self.segments >= MAX_SEGMENTS {
                return Err(CanvasError::limit("path"));
            }
            self.path.push(element);
            self.segments = self.segments.saturating_add(1);
        }
        self.bump();
        Ok(())
    }
}

fn connect_arc(current: Option<Point>, arc: &BezPath) -> Vec<PathEl> {
    let mut elements: Vec<PathEl> = arc.elements().to_vec();
    if let (Some(_), Some(PathEl::MoveTo(start))) = (current, elements.first().copied()) {
        elements[0] = PathEl::LineTo(start);
    }
    elements
}

pub(crate) fn normalize_rect(x: f64, y: f64, w: f64, h: f64) -> Option<(f64, f64, f64, f64)> {
    if ![x, y, w, h].iter().all(|value| finite(*value)) || w == 0.0 || h == 0.0 {
        return None;
    }
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    Some((x, y, w, h))
}

/// CSS corner order: top-left, top-right, bottom-right, bottom-left.
pub(crate) fn corner_radii(radii: &[f64]) -> [f64; 4] {
    match radii {
        [] => [0.0; 4],
        [all] => [*all; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d, ..] => [*a, *b, *c, *d],
    }
}

pub(crate) fn fit_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let radii = radii.map(|radius| radius.max(0.0));
    let sides = [
        radii[0] + radii[1],
        radii[2] + radii[3],
        radii[0] + radii[3],
        radii[1] + radii[2],
    ];
    let limits = [w, w, h, h];
    let scale = sides
        .iter()
        .zip(limits)
        .filter(|(side, _)| **side > 0.0)
        .map(|(side, limit)| limit / side)
        .fold(1.0, f64::min);
    radii.map(|radius| radius * scale)
}

pub(crate) fn arc_sweep(start: f64, end: f64, ccw: bool) -> f64 {
    const TAU: f64 = std::f64::consts::TAU;
    if !ccw {
        if end - start >= TAU {
            TAU
        } else {
            (end - start).rem_euclid(TAU)
        }
    } else if start - end >= TAU {
        -TAU
    } else {
        -(start - end).rem_euclid(TAU)
    }
}

pub(crate) fn transform_path(affine: &Affine, path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for element in path.elements() {
        out.push(match *element {
            PathEl::MoveTo(point) => PathEl::MoveTo(*affine * point),
            PathEl::LineTo(point) => PathEl::LineTo(*affine * point),
            PathEl::QuadTo(control, point) => PathEl::QuadTo(*affine * control, *affine * point),
            PathEl::CurveTo(c1, c2, point) => {
                PathEl::CurveTo(*affine * c1, *affine * c2, *affine * point)
            }
            PathEl::ClosePath => PathEl::ClosePath,
        });
    }
    out
}

pub(crate) fn invertible(affine: Affine) -> Option<Affine> {
    let det = affine.determinant();
    if !det.is_finite() || det.abs() < 1.0e-12 {
        None
    } else {
        let inverse = affine.inverse();
        inverse
            .as_coeffs()
            .iter()
            .all(|value| value.is_finite())
            .then_some(inverse)
    }
}

/// Stroke `device` (already in canvas space) using `ctm` for the pen scale.
pub(crate) fn stroke_outline(
    device: &BezPath,
    ctm: Affine,
    style: &StrokeSnap,
    tolerance: f64,
    deadline: &Deadline,
) -> Result<BezPath, CanvasError> {
    deadline.check()?;
    if style.width <= 0.0 || device.is_empty() {
        return Ok(BezPath::new());
    }
    let Some(inverse) = invertible(ctm) else {
        return Ok(BezPath::new());
    };
    let user = transform_path(&inverse, device);
    if !user.elements().iter().all(element_finite) {
        return Ok(BezPath::new());
    }
    deadline.check()?;
    let dashed = if style.dash.is_empty() {
        user
    } else {
        expand_dashes(&user, style, deadline)?
    };
    let pen = kurbo_stroke(style);
    let opts = kurbo::StrokeOpts::default();
    let tolerance = tolerance.max(1.0e-3);
    let mut output = BezPath::new();
    let mut scratch = kurbo::StrokeCtx::default();
    // Finish complete contours between budget checks so joins and caps still
    // belong to the same contour. Many short dashes cannot accumulate an
    // oversized outline in kurbo before the limit is checked.
    let contours = dashed
        .elements()
        .chunk_by(|_, next| !matches!(next, PathEl::MoveTo(_)));
    for contour in contours {
        deadline.check()?;
        if contour.len() <= 1 {
            continue;
        }
        let input = contour
            .iter()
            .copied()
            .take_while(|_| deadline.check().is_ok());
        kurbo::stroke_with(input, &pen, &opts, tolerance, &mut scratch);
        deadline.check()?;
        let outline = scratch.output().elements();
        if outline.len() > MAX_STROKE_VERBS.saturating_sub(output.elements().len()) {
            return Err(CanvasError::limit("stroke"));
        }
        for element in outline {
            let element = ctm * *element;
            if !element_finite(&element) {
                return Ok(BezPath::new());
            }
            output.push(element);
        }
    }
    Ok(output)
}

pub(crate) fn curve_segments(path: &BezPath) -> usize {
    if path.elements().is_empty() { 2 } else { 1 }
}

fn subpath_start(path: &BezPath) -> Option<Point> {
    path.elements()
        .iter()
        .rev()
        .find_map(|element| match *element {
            PathEl::MoveTo(point) => Some(point),
            _ => None,
        })
}

pub(crate) fn current_point(path: &BezPath) -> Option<Point> {
    match *path.elements().last()? {
        PathEl::ClosePath => subpath_start(path),
        last => last.end_point(),
    }
}

pub(crate) fn closing_start(path: &BezPath) -> Option<Point> {
    match path.elements().last()? {
        PathEl::MoveTo(_) | PathEl::ClosePath => None,
        _ => subpath_start(path),
    }
}

pub(crate) fn close_subpath(path: &mut BezPath, start: Point) {
    path.close_path();
    path.move_to(start);
}

/// `lineTo` with no subpath is a move to that point.
pub(crate) fn extend_line(path: &mut BezPath, point: Point) {
    if path.elements().is_empty() {
        path.move_to(point);
    } else {
        path.line_to(point);
    }
}

/// `quadraticCurveTo` with no subpath starts at the control point.
pub(crate) fn extend_quad(path: &mut BezPath, control: Point, point: Point) {
    if path.elements().is_empty() {
        path.move_to(control);
    }
    path.quad_to(control, point);
}

/// `bezierCurveTo` with no subpath starts at the first control point.
pub(crate) fn extend_curve(path: &mut BezPath, c1: Point, c2: Point, point: Point) {
    if path.elements().is_empty() {
        path.move_to(c1);
    }
    path.curve_to(c1, c2, point);
}

fn expand_dashes(
    path: &BezPath,
    style: &StrokeSnap,
    deadline: &Deadline,
) -> Result<BezPath, CanvasError> {
    let dashes = &style.dash;
    let period: f64 = dashes.iter().sum();
    if !period.is_finite() || period <= 0.0 {
        return Err(CanvasError::limit("stroke"));
    }
    // Kurbo can buffer the first dash of a closed contour before yielding it.
    // Bound dash transitions up front using the control polygons, whose lengths
    // bound the curves without adaptive arc-length integration.
    let subpaths = path
        .elements()
        .iter()
        .filter(|el| matches!(el, PathEl::MoveTo(_)))
        .count();
    let transitions = (subpaths as f64) * (dashes.len() as f64);
    let mut periods = 0.0;
    for segment in path.segments() {
        deadline.check()?;
        periods += control_polygon_length(segment) / period;
        if !periods.is_finite()
            || periods * dashes.len() as f64 + transitions > MAX_STROKE_VERBS as f64
        {
            return Err(CanvasError::limit("stroke"));
        }
    }
    let mut out = BezPath::new();
    let input = path.iter().take_while(|_| deadline.check().is_ok());
    for element in kurbo::dash(input, style.dash_offset, dashes) {
        deadline.check()?;
        if out.elements().len() >= MAX_STROKE_VERBS {
            return Err(CanvasError::limit("stroke"));
        }
        out.push(element);
    }
    deadline.check()?;
    Ok(out)
}

fn control_polygon_length(segment: PathSeg) -> f64 {
    match segment {
        PathSeg::Line(line) => line.p0.distance(line.p1),
        PathSeg::Quad(quad) => quad.p0.distance(quad.p1) + quad.p1.distance(quad.p2),
        PathSeg::Cubic(cubic) => {
            cubic.p0.distance(cubic.p1) + cubic.p1.distance(cubic.p2) + cubic.p2.distance(cubic.p3)
        }
    }
}

fn element_finite(element: &PathEl) -> bool {
    match *element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => point_finite(point),
        PathEl::QuadTo(control, point) => point_finite(control) && point_finite(point),
        PathEl::CurveTo(c1, c2, point) => {
            point_finite(c1) && point_finite(c2) && point_finite(point)
        }
        PathEl::ClosePath => true,
    }
}

fn point_finite(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

pub(crate) fn closed_for_fill(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for element in path.elements() {
        if matches!(element, PathEl::MoveTo(_)) && open {
            out.close_path();
        }
        out.push(*element);
        open = !matches!(element, PathEl::ClosePath);
    }
    if open {
        out.close_path();
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

pub(crate) fn contains_fill(path: &BezPath, point: Point, rule: FillRule) -> bool {
    if !point_finite(point) {
        return false;
    }
    let winding = closed_for_fill(path).winding(point);
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding.abs() % 2 == 1,
    }
}

pub(crate) fn contains_stroke(
    device: &BezPath,
    ctm: Affine,
    style: &StrokeSnap,
    point: Point,
    tolerance: f64,
    deadline: &Deadline,
) -> Result<bool, CanvasError> {
    let outline = stroke_outline(device, ctm, style, tolerance, deadline)?;
    Ok(contains_fill(&outline, point, FillRule::NonZero))
}

fn tangent_arc(current: Point, corner: Point, next: Point, radius: f64) -> (BezPath, Point) {
    let mut path = BezPath::new();
    if radius == 0.0 {
        path.move_to(current);
        path.line_to(corner);
        return (path, corner);
    }
    let v1 = current - corner;
    let v2 = next - corner;
    let len1 = v1.hypot();
    let len2 = v2.hypot();
    if len1 < 1.0e-9 || len2 < 1.0e-9 {
        path.move_to(current);
        path.line_to(corner);
        return (path, corner);
    }
    let u1 = v1 / len1;
    let u2 = v2 / len2;
    let dot = (u1.dot(u2)).clamp(-1.0, 1.0);
    let angle = dot.acos();
    if angle < 1.0e-6 || (std::f64::consts::PI - angle).abs() < 1.0e-6 {
        path.move_to(current);
        path.line_to(corner);
        return (path, corner);
    }
    let half = angle * 0.5;
    let dist = radius / half.tan();
    let start = corner + u1 * dist;
    let end = corner + u2 * dist;
    let bis = Vec2::new(u1.x + u2.x, u1.y + u2.y);
    let bis_len = bis.hypot();
    if bis_len < 1.0e-9 {
        path.move_to(current);
        path.line_to(corner);
        return (path, corner);
    }
    let center = corner + (bis / bis_len) * (radius / half.sin());
    let start_angle = (start.y - center.y).atan2(start.x - center.x);
    let end_angle = (end.y - center.y).atan2(end.x - center.x);
    let mut sweep = end_angle - start_angle;
    if sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    } else if sweep < -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    path.move_to(current);
    path.line_to(start);
    let arc = kurbo::Arc::new(center, (radius, radius), start_angle, sweep, 0.0);
    let mut skipped = false;
    for element in arc.to_path(0.25).elements() {
        if !skipped {
            skipped = true;
            continue;
        }
        path.push(*element);
    }
    (path, end)
}
