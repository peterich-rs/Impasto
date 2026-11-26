//! Compile SVG artwork into lazily constructed Canvas paths.
use std::{collections::HashSet, fmt::Write as _, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tiny_skia_path::{PathSegment, Point, Rect, Transform};
use usvg::{BlendMode, FillRule, Group, LineCap, LineJoin, Node, Paint, PaintOrder};

pub(crate) fn compile(path: &Path) -> Result<String> {
    let source = fs::read_to_string(path)?;
    compile_source(&source).with_context(|| format!("{}: unsupported SVG icon", path.display()))
}

fn compile_source(source: &str) -> Result<String> {
    let document = roxmltree::Document::parse(source)?;
    let elements = document.descendants().filter(roxmltree::Node::is_element);
    if elements.clone().any(|node| {
        matches!(
            node.tag_name().name(),
            "text" | "image" | "foreignObject" | "script"
        )
    }) {
        bail!("SVG icons must use vector paths and shapes");
    }
    let tree = usvg::Tree::from_str(source, &usvg::Options::default())?;
    let marker = color_marker(tree.root());
    let mut substitutions = Vec::new();
    for node in elements {
        for attr in node
            .attributes()
            .filter(|attr| matches!(attr.name(), "fill" | "stroke" | "stop-color" | "style"))
        {
            let range = attr.range_value();
            let text = &source[range.clone()];
            if let Some(changed) = mark_current_color(text, &marker) {
                substitutions.push((range, changed));
            }
        }
        if node.has_tag_name("style") {
            for child in node.children().filter(roxmltree::Node::is_text) {
                let range = child.range();
                if let Some(changed) = mark_current_color(&source[range.clone()], &marker) {
                    substitutions.push((range, changed));
                }
            }
        }
    }
    substitutions.sort_by_key(|(range, _)| range.start);
    let marked = if substitutions.is_empty() {
        None
    } else {
        let mut replaced = source.to_owned();
        for (range, value) in substitutions.into_iter().rev() {
            replaced.replace_range(range, &value);
        }
        Some(usvg::Tree::from_str(&replaced, &usvg::Options::default())?)
    };
    let mut operations = Vec::new();
    emit_group(
        tree.root(),
        marked.as_ref().map(usvg::Tree::root),
        &mut operations,
    )?;
    let size = tree.size();
    let mut shared = String::new();
    if let Some(stroke) = common_stroke(&mut operations) {
        if solid_stroke(&stroke)
            && operations
                .iter()
                .all(|operation| operation.get(1).is_none())
        {
            let paths = operations
                .iter()
                .filter_map(|operation| operation.get(0))
                .collect::<Vec<_>>();
            return Ok(format!(
                "import{{strokeAsset as v}}from'@impasto/solid-renderer/vector';export default v({},{},{},{stroke});",
                size.width(),
                size.height(),
                json!(paths),
            ));
        }
        write!(shared, ",{stroke}")?;
    }
    Ok(format!(
        "import{{vectorAsset as v}}from'@impasto/solid-renderer/vector';export default v({},{},{}{shared});",
        size.width(),
        size.height(),
        Value::Array(operations),
    ))
}

fn solid_stroke(stroke: &Value) -> bool {
    stroke
        .get(0)
        .is_some_and(|paint| paint.is_string() || paint.get(0).is_some_and(Value::is_string))
}

fn common_stroke(operations: &mut [Value]) -> Option<Value> {
    let stroke = operations.first()?.get(2)?.clone();
    if stroke.is_null()
        || !operations
            .iter()
            .all(|operation| operation.get(2) == Some(&stroke))
    {
        return None;
    }
    for operation in operations.iter_mut().filter_map(Value::as_array_mut) {
        if operation.len() == 3 && operation[1].is_null() {
            operation.truncate(1);
        } else {
            operation[2] = Value::Null;
        }
    }
    Some(stroke)
}

fn color_marker(root: &Group) -> String {
    let mut colors = HashSet::new();
    let mut groups = vec![root];
    while let Some(group) = groups.pop() {
        for node in group.children() {
            match node {
                Node::Group(group) => groups.push(group),
                Node::Path(path) => {
                    let fill = path.fill().map(usvg::Fill::paint);
                    let stroke = path.stroke().map(usvg::Stroke::paint);
                    for paint in fill.into_iter().chain(stroke) {
                        if let Paint::Color(color) = paint {
                            colors.insert(u32::from_be_bytes([
                                0,
                                color.red,
                                color.green,
                                color.blue,
                            ]));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let start = 0x01_97e3;
    let marker = (start..=0xff_ffff)
        .chain(0..start)
        .find(|value| !colors.contains(value))
        .unwrap_or(start);
    format!("#{marker:06x}")
}

fn mark_current_color(text: &str, marker: &str) -> Option<String> {
    let mut marked = text.to_owned();
    let mut changed = false;
    for (index, word) in text.to_ascii_lowercase().rmatch_indices("currentcolor") {
        let end = index + word.len();
        let part = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'#');
        if index
            .checked_sub(1)
            .is_some_and(|i| part(text.as_bytes()[i]))
            || text.as_bytes().get(end).copied().is_some_and(part)
        {
            continue;
        }
        marked.replace_range(index..end, marker);
        changed = true;
    }
    changed.then_some(marked)
}

fn emit_group(group: &Group, marked: Option<&Group>, operations: &mut Vec<Value>) -> Result<()> {
    if group.opacity().get() < 1.0
        || group.blend_mode() != BlendMode::Normal
        || group.clip_path().is_some()
        || group.mask().is_some()
        || !group.filters().is_empty()
    {
        bail!("SVG group opacity, blending, clips, masks and filters require an image asset");
    }
    let mut siblings = group.children().iter().enumerate().peekable();
    while let Some((index, node)) = siblings.next() {
        let probe = marked.and_then(|group| group.children().get(index));
        let path = match node {
            Node::Group(group) => {
                let probe = if let Some(Node::Group(group)) = probe {
                    Some(group.as_ref())
                } else {
                    None
                };
                emit_group(group, probe, operations)?;
                continue;
            }
            Node::Path(path) if path.is_visible() => path,
            Node::Path(_) => continue,
            _ => bail!("SVG icons must use vector paths and shapes"),
        };
        let probe = probe_path(probe);
        let transform = path.abs_transform();
        // A stroke's pen and dash lengths are defined in the original coordinate space.
        let bake = path.stroke().is_none()
            && path
                .fill()
                .is_none_or(|fill| matches!(fill.paint(), Paint::Color(_)));
        let mut data = String::new();
        append_path(&mut data, path, bake)?;
        if let Some(key) = solid_fill_key(path, probe) {
            let mut bounds = path.abs_bounding_box();
            while let Some(&(index, Node::Path(next))) = siblings.peek() {
                let next_probe = probe_path(marked.and_then(|group| group.children().get(index)));
                let next_bounds = next.abs_bounding_box();
                // Merging overlapping fills can change winding, opacity and antialiasing.
                if solid_fill_key(next, next_probe) != Some(key) || !disjoint(bounds, next_bounds) {
                    break;
                }
                let Some(joined) = Rect::from_ltrb(
                    bounds.left().min(next_bounds.left()),
                    bounds.top().min(next_bounds.top()),
                    bounds.right().max(next_bounds.right()),
                    bounds.bottom().max(next_bounds.bottom()),
                ) else {
                    break;
                };
                bounds = joined;
                append_path(&mut data, next, true)?;
                siblings.next();
            }
        }
        let fill = path
            .fill()
            .map(|style| {
                let probe = probe.and_then(usvg::Path::fill).map(usvg::Fill::paint);
                let paint = paint_descriptor(style.paint(), probe)?;
                Ok::<_, anyhow::Error>(json!([
                    paint,
                    style.opacity().get(),
                    style.rule() == FillRule::EvenOdd
                ]))
            })
            .transpose()?;
        let stroke = path
            .stroke()
            .map(|style| stroke_descriptor(style, probe))
            .transpose()?;
        let scoped = !bake && !transform.is_identity();
        let stroke_first = path.paint_order() == PaintOrder::StrokeAndFill;
        let mut operation = vec![json!(data), json!(fill), json!(stroke)];
        if scoped || stroke_first {
            operation.push(if scoped {
                json!([
                    transform.sx,
                    transform.ky,
                    transform.kx,
                    transform.sy,
                    transform.tx,
                    transform.ty
                ])
            } else {
                Value::Null
            });
        }
        if stroke_first {
            operation.push(json!(true));
        }
        operations.push(Value::Array(operation));
    }
    Ok(())
}

fn probe_path(node: Option<&Node>) -> Option<&usvg::Path> {
    match node {
        Some(Node::Path(path)) => Some(path.as_ref()),
        _ => None,
    }
}

fn solid_fill_key(
    path: &usvg::Path,
    probe: Option<&usvg::Path>,
) -> Option<(usvg::Color, f32, FillRule, bool)> {
    let fill = path.fill()?;
    let Paint::Color(color) = fill.paint() else {
        return None;
    };
    let dynamic = matches!(probe.and_then(usvg::Path::fill).map(usvg::Fill::paint), Some(Paint::Color(marked)) if color != marked);
    (path.is_visible() && path.stroke().is_none()).then_some((
        *color,
        fill.opacity().get(),
        fill.rule(),
        dynamic,
    ))
}

fn disjoint(first: Rect, second: Rect) -> bool {
    first.right() < second.left()
        || second.right() < first.left()
        || first.bottom() < second.top()
        || second.bottom() < first.top()
}

fn append_path(data: &mut String, path: &usvg::Path, bake: bool) -> Result<()> {
    let transform = path.abs_transform();
    let mut command = |verb, points: &[Point]| -> Result<()> {
        data.push(verb);
        for (index, mut point) in points.iter().copied().enumerate() {
            if index != 0 {
                data.push(' ');
            }
            if bake {
                transform.map_point(&mut point);
            }
            if bake || transform.is_identity() {
                write!(data, "{} {}", coordinate(point.x), coordinate(point.y))?;
            } else {
                write!(data, "{} {}", point.x, point.y)?;
            }
        }
        Ok(())
    };
    for segment in path.data().segments() {
        match segment {
            PathSegment::MoveTo(p) => command('M', &[p])?,
            PathSegment::LineTo(p) => command('L', &[p])?,
            PathSegment::QuadTo(a, b) => command('Q', &[a, b])?,
            PathSegment::CubicTo(a, b, c) => command('C', &[a, b, c])?,
            PathSegment::Close => command('Z', &[])?,
        }
    }
    Ok(())
}

fn coordinate(value: f32) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if value != 0.0 && value.abs() < 0.01 {
        return value.to_string();
    }
    let number = format!("{value:.2}");
    if number.contains('.') {
        number
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else {
        number
    }
}

fn stroke_descriptor(style: &usvg::Stroke, probe: Option<&usvg::Path>) -> Result<Value> {
    let probe = probe.and_then(usvg::Path::stroke).map(usvg::Stroke::paint);
    let paint = paint_descriptor(style.paint(), probe)?;
    let cap = match style.linecap() {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    };
    let join = match style.linejoin() {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
        LineJoin::MiterClip => bail!("Canvas cannot render miter-clip joins"),
    };
    let mut style_data = vec![
        paint,
        json!(style.opacity().get()),
        json!(style.width().get()),
        json!(cap),
        json!(join),
        json!(style.miterlimit().get()),
    ];
    if style.dasharray().is_some() || style.dashoffset() != 0.0 {
        style_data.push(json!(style.dasharray().unwrap_or_default()));
        style_data.push(json!(style.dashoffset()));
    }
    Ok(Value::Array(style_data))
}

fn paint_descriptor(paint: &Paint, probe: Option<&Paint>) -> Result<Value> {
    let (mut data, stops) = match paint {
        Paint::Color(color) => {
            let fallback = format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue);
            let dynamic = matches!(probe, Some(Paint::Color(marked)) if color != marked);
            return Ok(if dynamic {
                json!([fallback])
            } else {
                json!(fallback)
            });
        }
        Paint::LinearGradient(gradient) => {
            let a = Point::from_xy(gradient.x1(), gradient.y1());
            let b = Point::from_xy(gradient.x2(), gradient.y2());
            let (a, b) = linear_gradient_axis(a, b, gradient.transform());
            (
                vec![json!(0), json!(a.x), json!(a.y), json!(b.x), json!(b.y)],
                gradient.stops(),
            )
        }
        Paint::RadialGradient(gradient) => {
            let mut points = [
                Point::from_xy(gradient.fx(), gradient.fy()),
                Point::from_xy(gradient.cx(), gradient.cy()),
            ];
            let t = gradient.transform();
            t.map_points(&mut points);
            let [a, b] = points;
            let radius = gradient.r().get() * (t.sx * t.sy - t.kx * t.ky).abs().sqrt();
            (
                vec![
                    json!(1),
                    json!(a.x),
                    json!(a.y),
                    json!(b.x),
                    json!(b.y),
                    json!(radius),
                ],
                gradient.stops(),
            )
        }
        Paint::Pattern(_) => bail!("SVG patterns require an image asset"),
    };
    let stops = stops
        .iter()
        .map(|stop| {
            json!([
                stop.offset().get(),
                rgba(stop.color(), stop.opacity().get())
            ])
        })
        .collect();
    data.push(Value::Array(stops));
    Ok(Value::Array(data))
}

fn linear_gradient_axis(a: Point, b: Point, transform: Transform) -> (Point, Point) {
    let mut points = [a, b];
    transform.map_points(&mut points);
    if let Some(inverse) = transform.invert() {
        // Preserve the iso-color lines under shear and non-uniform scale.
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let x = inverse.sx * dx + inverse.ky * dy;
        let y = inverse.kx * dx + inverse.sy * dy;
        let scale = (dx * dx + dy * dy) / (x * x + y * y);
        if scale.is_finite() {
            points[1] = Point::from_xy(points[0].x + x * scale, points[0].y + y * scale);
        }
    }
    (points[0], points[1])
}

fn rgba(color: usvg::Color, alpha: f32) -> String {
    format!("rgba({},{},{},{alpha})", color.red, color.green, color.blue)
}
