//! Compiles flat SVG artwork to Canvas path instructions.

use std::{fmt::Write as _, fs, path::Path};

use anyhow::{Context, Result, bail};
use tiny_skia_path::PathSegment;
use usvg::{BlendMode, FillRule, Group, LineCap, LineJoin, Node, Paint, PaintOrder};

pub(crate) fn compile(path: &Path) -> Result<String> {
    let source = fs::read_to_string(path)?;
    compile_source(&source).with_context(|| format!("{}: unsupported SVG icon", path.display()))
}

fn compile_source(source: &str) -> Result<String> {
    let parsed = roxmltree::Document::parse(source)?;
    if parsed.descendants().any(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "text" | "image" | "foreignObject" | "script"
            )
    }) {
        bail!("SVG icons must use vector paths and shapes");
    }
    let tree = usvg::Tree::from_str(source, &usvg::Options::default())?;
    let mut code = String::from("import { scene } from 'scene';\n");
    let mut draw = String::new();
    let mut count = 0;
    emit_group(tree.root(), &mut code, &mut draw, &mut count)?;
    let size = tree.size();
    writeln!(
        code,
        "export default {{ width: {}, height: {}, draw(ctx) {{ {draw} }} }};",
        size.width(),
        size.height()
    )?;
    Ok(code)
}

fn emit_group(
    group: &Group,
    code: &mut String,
    draw: &mut String,
    count: &mut usize,
) -> Result<()> {
    if group.opacity().get() < 1.0
        || group.blend_mode() != BlendMode::Normal
        || group.clip_path().is_some()
        || group.mask().is_some()
        || !group.filters().is_empty()
    {
        bail!("SVG group opacity, blending, clips, masks and filters require an image asset");
    }
    for node in group.children() {
        let path = match node {
            Node::Group(group) => {
                emit_group(group, code, draw, count)?;
                continue;
            }
            Node::Path(path) if path.is_visible() => path,
            Node::Path(_) => continue,
            _ => bail!("SVG icons must use vector paths and shapes"),
        };
        let name = format!("path{count}");
        *count += 1;
        writeln!(code, "const {name} = new scene.Path2D();")?;
        for command in path.data().segments() {
            match command {
                PathSegment::MoveTo(p) => writeln!(code, "{name}.moveTo({}, {});", p.x, p.y)?,
                PathSegment::LineTo(p) => writeln!(code, "{name}.lineTo({}, {});", p.x, p.y)?,
                PathSegment::QuadTo(a, b) => writeln!(
                    code,
                    "{name}.quadraticCurveTo({}, {}, {}, {});",
                    a.x, a.y, b.x, b.y
                )?,
                PathSegment::CubicTo(a, b, c) => writeln!(
                    code,
                    "{name}.bezierCurveTo({}, {}, {}, {}, {}, {});",
                    a.x, a.y, b.x, b.y, c.x, c.y
                )?,
                PathSegment::Close => writeln!(code, "{name}.closePath();")?,
            }
        }
        let t = path.abs_transform();
        writeln!(
            draw,
            "ctx.save(); ctx.transform({}, {}, {}, {}, {}, {});",
            t.sx, t.ky, t.kx, t.sy, t.tx, t.ty
        )?;
        let mut fill = String::new();
        if let Some(style) = path.fill() {
            let color = color(style.paint(), style.opacity().get())?;
            let rule = match style.rule() {
                FillRule::NonZero => "nonzero",
                FillRule::EvenOdd => "evenodd",
            };
            writeln!(
                fill,
                "ctx.fillStyle = {color:?}; ctx.fill({name}, {rule:?});"
            )?;
        }
        let mut stroke = String::new();
        if let Some(style) = path.stroke() {
            let color = color(style.paint(), style.opacity().get())?;
            let cap = match style.linecap() {
                LineCap::Butt => "butt",
                LineCap::Round => "round",
                LineCap::Square => "square",
            };
            let join = match style.linejoin() {
                LineJoin::Miter => "miter",
                LineJoin::Round => "round",
                LineJoin::Bevel => "bevel",
                LineJoin::MiterClip => bail!("Canvas cannot render miter-clip joins"),
            };
            writeln!(
                stroke,
                "ctx.strokeStyle = {color:?}; ctx.lineWidth = {}; ctx.lineCap = {cap:?}; ctx.lineJoin = {join:?}; ctx.miterLimit = {}; ctx.setLineDash({:?}); ctx.lineDashOffset = {}; ctx.stroke({name});",
                style.width().get(),
                style.miterlimit().get(),
                style.dasharray().unwrap_or_default(),
                style.dashoffset()
            )?;
        }
        match path.paint_order() {
            PaintOrder::FillAndStroke => writeln!(draw, "{fill}{stroke}")?,
            PaintOrder::StrokeAndFill => writeln!(draw, "{stroke}{fill}")?,
        }
        writeln!(draw, "ctx.restore();")?;
    }
    Ok(())
}

fn color(paint: &Paint, opacity: f32) -> Result<String> {
    let Paint::Color(color) = paint else {
        bail!("SVG icons must use solid fills and strokes");
    };
    Ok(format!(
        "rgba({}, {}, {}, {opacity})",
        color.red, color.green, color.blue
    ))
}
