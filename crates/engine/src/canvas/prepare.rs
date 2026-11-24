//! Turn a recording into an immutable [`crate::scene::CanvasList`].

use std::mem::{size_of, size_of_val};
use std::sync::Arc;

use crate::scene::{
    CanvasBrush, CanvasComposite, CanvasImageOp, CanvasImageRef, CanvasIsolation, CanvasList,
    CanvasOp, CanvasPaint, CanvasPath, CanvasTextOp, ColorStop as ProtoStop, PathVerb,
};
use kurbo::{Affine, BezPath, PathEl, Point};

use super::paint::{GradientKind, PaintSource, text_unavailable};
use super::path::{FillRule, closed_for_fill, stroke_outline};
use super::record::{self, Deadline, RecOp, Recording, TextAlign, TextBaseline};
use super::text::flatten_text;
use super::{CURVE_PIXELS, CanvasError, MAX_OPS, MAX_RECORD_BYTES, MAX_SEGMENTS, narrow};

pub fn prepare(
    mut recording: Recording<'_>,
    shaper: &mut dyn crate::text::Shaper,
    deadline: &Deadline,
) -> Result<CanvasList, CanvasError> {
    let list = build(&recording, shaper, deadline)?;
    let budget = recording.budget;
    recording.release_charge();
    budget.track(&list)?;
    Ok(list)
}

fn build(
    recording: &Recording<'_>,
    shaper: &mut dyn crate::text::Shaper,
    deadline: &Deadline,
) -> Result<CanvasList, CanvasError> {
    deadline.check()?;
    let tolerance = tolerance(recording.pixel_ratio);
    let mut ops = Vec::new();
    let mut images = Vec::new();
    let mut isolated = false;
    let mut segments = 0_usize;
    for op in &recording.ops {
        deadline.check()?;
        if ops.len() >= MAX_OPS {
            return Err(CanvasError::limit("prepare"));
        }
        match op {
            RecOp::Save => ops.push(CanvasOp::Save),
            RecOp::Restore => ops.push(CanvasOp::Restore),
            RecOp::Clip { path, fill } => {
                let path =
                    path_of(&closed_for_fill(path), *fill, &mut segments)?.unwrap_or_else(|| {
                        CanvasPath {
                            verbs: Arc::from([]),
                            fill: protocol_fill(*fill),
                        }
                    });
                ops.push(CanvasOp::Clip { path });
            }
            RecOp::Fill {
                path,
                fill,
                paint,
                alpha,
                composite,
                ctm,
            } => {
                if composite.reads_destination() {
                    isolated = true;
                }
                let Some(path) = path_of(&closed_for_fill(path), *fill, &mut segments)? else {
                    continue;
                };
                let Some(paint) = paint_of(paint, *alpha, *composite, *ctm) else {
                    continue;
                };
                ops.push(CanvasOp::Fill { path, paint });
            }
            RecOp::Stroke {
                path,
                style,
                paint,
                alpha,
                composite,
                ctm,
            } => {
                if composite.reads_destination() {
                    isolated = true;
                }
                let outline = stroke_outline(path, *ctm, style, tolerance, deadline)?;
                let Some(path) = path_of(&outline, FillRule::NonZero, &mut segments)? else {
                    continue;
                };
                let Some(paint) = paint_of(paint, *alpha, *composite, *ctm) else {
                    continue;
                };
                ops.push(CanvasOp::Fill { path, paint });
            }
            RecOp::Clear { x, y, w, h, ctm } => {
                isolated = true;
                if let Some(op) = clear_of(*x, *y, *w, *h, *ctm) {
                    ops.push(op);
                }
            }
            RecOp::Text {
                text,
                x,
                y,
                max_width,
                align,
                baseline,
                direction_rtl,
                font,
                paint,
                alpha,
                composite,
                ctm,
            } => {
                if composite.reads_destination() {
                    isolated = true;
                }
                if let Some(op) = text_of(
                    text,
                    *x,
                    *y,
                    *max_width,
                    *align,
                    *baseline,
                    *direction_rtl,
                    font,
                    paint,
                    *alpha,
                    *composite,
                    *ctm,
                    shaper,
                )? {
                    ops.push(CanvasOp::Text(op));
                }
            }
            RecOp::Image {
                source,
                src,
                x,
                y,
                w,
                h,
                smoothing,
                alpha,
                composite,
                ctm,
            } => {
                if composite.reads_destination() {
                    isolated = true;
                }
                if let Some(op) = image_of(
                    source, *src, *x, *y, *w, *h, *smoothing, *alpha, *composite, *ctm,
                ) {
                    note_image(&mut images, &op.source);
                    ops.push(CanvasOp::Image(op));
                }
            }
        }
    }
    if ops.len() > MAX_OPS || segments > MAX_SEGMENTS {
        return Err(CanvasError::limit("prepare"));
    }
    let images: Box<[CanvasImageRef]> = images.into();
    let ops: Box<[CanvasOp]> = ops.into();
    let bytes = list_bytes(&ops, &images);
    if bytes > MAX_RECORD_BYTES {
        return Err(CanvasError::limit("prepare"));
    }
    Ok(CanvasList {
        body: Arc::new(crate::scene::CanvasBody {
            ops,
            images,
            isolation: if isolated {
                CanvasIsolation::Isolated
            } else {
                CanvasIsolation::Shared
            },
            bytes,
        }),
    })
}

fn path_of(
    path: &BezPath,
    fill: FillRule,
    segments: &mut usize,
) -> Result<Option<CanvasPath>, CanvasError> {
    if path.is_empty() {
        return Ok(None);
    }
    let mut verbs = Vec::new();
    for element in path.elements() {
        if *segments >= MAX_SEGMENTS {
            return Err(CanvasError::limit("prepare"));
        }
        let Some(verb) = verb_of(*element) else {
            return Ok(None);
        };
        verbs.push(verb);
        *segments = segments.saturating_add(1);
    }
    if verbs.is_empty() {
        return Ok(None);
    }
    Ok(Some(CanvasPath {
        verbs: Arc::from(verbs),
        fill: protocol_fill(fill),
    }))
}

fn protocol_fill(fill: FillRule) -> crate::scene::FillRule {
    match fill {
        FillRule::NonZero => crate::scene::FillRule::NonZero,
        FillRule::EvenOdd => crate::scene::FillRule::EvenOdd,
    }
}

pub(crate) fn verb_of(element: PathEl) -> Option<PathVerb> {
    match element {
        PathEl::MoveTo(point) => Some(PathVerb::Move(narrow(point.x)?, narrow(point.y)?)),
        PathEl::LineTo(point) => Some(PathVerb::Line(narrow(point.x)?, narrow(point.y)?)),
        PathEl::QuadTo(control, point) => Some(PathVerb::Quad(
            narrow(control.x)?,
            narrow(control.y)?,
            narrow(point.x)?,
            narrow(point.y)?,
        )),
        PathEl::CurveTo(c1, c2, point) => Some(PathVerb::Cubic(
            narrow(c1.x)?,
            narrow(c1.y)?,
            narrow(c2.x)?,
            narrow(c2.y)?,
            narrow(point.x)?,
            narrow(point.y)?,
        )),
        PathEl::ClosePath => Some(PathVerb::Close),
    }
}

fn paint_of(
    source: &PaintSource,
    alpha: f64,
    composite: CanvasComposite,
    ctm: Affine,
) -> Option<CanvasPaint> {
    let brush = match source {
        PaintSource::Color(color) => CanvasBrush::Color(*color),
        PaintSource::Gradient(gradient) => {
            if gradient.stops.is_empty() {
                CanvasBrush::Color([0, 0, 0, 0])
            } else {
                brush_gradient(gradient, ctm)?
            }
        }
    };
    Some(CanvasPaint {
        brush,
        alpha: record::alpha_of(alpha),
        composite,
    })
}

fn brush_gradient(gradient: &super::paint::GradientSnap, ctm: Affine) -> Option<CanvasBrush> {
    let stops = proto_stops(&gradient.stops)?;
    match gradient.kind {
        GradientKind::Linear { x0, y0, x1, y1 } => {
            let start = ctm * Point::new(x0, y0);
            let end = ctm * Point::new(x1, y1);
            if (start - end).hypot() < 1.0e-6 {
                return None;
            }
            Some(CanvasBrush::Linear {
                start: [narrow(start.x)?, narrow(start.y)?],
                end: [narrow(end.x)?, narrow(end.y)?],
                stops,
            })
        }
        GradientKind::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
        } => {
            let start = ctm * Point::new(x0, y0);
            let end = ctm * Point::new(x1, y1);
            let scale = average_scale(ctm);
            let start_radius = r0 * scale;
            let end_radius = r1 * scale;
            if (start - end).hypot() < 1.0e-6 && (start_radius - end_radius).abs() < 1.0e-6 {
                return None;
            }
            Some(CanvasBrush::Radial {
                start: [narrow(start.x)?, narrow(start.y)?],
                start_radius: narrow(start_radius)?,
                end: [narrow(end.x)?, narrow(end.y)?],
                end_radius: narrow(end_radius)?,
                stops,
            })
        }
        GradientKind::Conic { angle, x, y } => {
            let center = ctm * Point::new(x, y);
            let coeffs = ctm.as_coeffs();
            let rotation = coeffs[1].atan2(coeffs[0]);
            Some(CanvasBrush::Conic {
                center: [narrow(center.x)?, narrow(center.y)?],
                angle: narrow(angle + rotation)?,
                stops,
            })
        }
    }
}

fn proto_stops(stops: &[super::paint::ColorStop]) -> Option<Arc<[ProtoStop]>> {
    let mut out = Vec::with_capacity(stops.len());
    for stop in stops {
        out.push(ProtoStop::srgb(narrow(stop.offset)?, stop.color));
    }
    Some(Arc::from(out))
}

fn clear_of(x: f64, y: f64, w: f64, h: f64, ctm: Affine) -> Option<CanvasOp> {
    Some(CanvasOp::Clear {
        x: narrow(x)?,
        y: narrow(y)?,
        w: narrow(w)?,
        h: narrow(h)?,
        ctm: narrow_affine(ctm)?,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "one recorded text op is unpacked so prepare can resolve metrics without cloning the string"
)]
fn text_of(
    text: &str,
    x: f64,
    y: f64,
    max_width: Option<f64>,
    align: TextAlign,
    baseline: TextBaseline,
    direction_rtl: bool,
    font: &super::text::CanvasFont,
    paint: &PaintSource,
    alpha: f64,
    composite: CanvasComposite,
    ctm: Affine,
    shaper: &mut dyn crate::text::Shaper,
) -> Result<Option<CanvasTextOp>, CanvasError> {
    let flat = flatten_text(text);
    if flat.is_empty() {
        return Ok(None);
    }
    let style = font.text_style(direction_rtl);
    let Some(font_size) = narrow(font.size).filter(|size| *size > 0.0) else {
        return Ok(None);
    };
    // One unbreakable layout serves the anchor, the font box, and the glyphs.
    let layout = shaper
        .shape_canvas(&flat, &style, font_size)
        .ok_or_else(|| text_unavailable("fillText"))?;
    let metrics = layout.metrics;
    let width = f64::from(metrics.width);
    let ascent = f64::from(metrics.ascent);
    let descent = f64::from(metrics.descent);
    let mut compress = 1.0;
    if let Some(limit) = max_width.filter(|limit| *limit > 0.0)
        && width > limit
        && width > 0.0
    {
        compress = limit / width;
    }
    let anchor_x = x;
    let left = match resolved_align(align, direction_rtl) {
        TextAlign::Left | TextAlign::Start => anchor_x,
        TextAlign::Center => anchor_x - width * 0.5,
        TextAlign::Right | TextAlign::End => anchor_x - width,
    };
    let em = ascent + descent;
    let top = match baseline {
        TextBaseline::Alphabetic => y - ascent,
        TextBaseline::Top => y,
        TextBaseline::Middle => y - em * 0.5,
        TextBaseline::Bottom | TextBaseline::Ideographic => y - em,
    };
    let Some(paint) = paint_of(paint, alpha, composite, ctm) else {
        return Ok(None);
    };
    let Some(ctm) = narrow_affine(ctm) else {
        return Ok(None);
    };
    let (Some(x), Some(y), Some(anchor_x), Some(compress)) = (
        narrow(left),
        narrow(top),
        narrow(anchor_x),
        narrow(compress),
    ) else {
        return Ok(None);
    };
    Ok(Some(CanvasTextOp {
        x,
        y,
        anchor_x,
        utf8: Arc::from(flat),
        compress,
        ctm,
        shaped: layout.shaped,
        paint,
    }))
}

fn resolved_align(align: TextAlign, rtl: bool) -> TextAlign {
    match align {
        TextAlign::Start => {
            if rtl {
                TextAlign::Right
            } else {
                TextAlign::Left
            }
        }
        TextAlign::End => {
            if rtl {
                TextAlign::Left
            } else {
                TextAlign::Right
            }
        }
        other => other,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one recorded image op is unpacked into the protocol image command"
)]
fn image_of(
    source: &str,
    src: Option<[f64; 4]>,
    x: f64,
    y: f64,
    w: Option<f64>,
    h: Option<f64>,
    smoothing: bool,
    alpha: f64,
    composite: CanvasComposite,
    ctm: Affine,
) -> Option<CanvasImageOp> {
    let src = match src {
        Some(rect) => Some([
            narrow(rect[0])?,
            narrow(rect[1])?,
            narrow(rect[2])?,
            narrow(rect[3])?,
        ]),
        None => None,
    };
    Some(CanvasImageOp {
        source: Arc::from(source),
        src,
        x: narrow(x)?,
        y: narrow(y)?,
        w: w.and_then(narrow),
        h: h.and_then(narrow),
        smoothing,
        alpha: record::alpha_of(alpha),
        composite,
        ctm: narrow_affine(ctm)?,
    })
}

fn note_image(images: &mut Vec<CanvasImageRef>, source: &Arc<str>) {
    if images
        .iter()
        .any(|image| image.source.as_ref() == source.as_ref())
    {
        return;
    }
    images.push(CanvasImageRef {
        source: Arc::clone(source),
    });
}

fn narrow_affine(affine: Affine) -> Option<[f32; 6]> {
    let mut out = [0.0_f32; 6];
    for (slot, value) in out.iter_mut().zip(affine.as_coeffs()) {
        *slot = narrow(value)?;
    }
    Some(out)
}

fn average_scale(affine: Affine) -> f64 {
    let [a, b, c, d, _, _] = affine.as_coeffs();
    let sx = a.hypot(b);
    let sy = c.hypot(d);
    (sx + sy) * 0.5
}

fn tolerance(pixel_ratio: f64) -> f64 {
    let ratio = if pixel_ratio.is_finite() && pixel_ratio > 0.0 {
        pixel_ratio
    } else {
        1.0
    };
    (CURVE_PIXELS / ratio).max(1.0e-4)
}

fn list_bytes(ops: &[CanvasOp], images: &[CanvasImageRef]) -> usize {
    let mut bytes = size_of::<crate::scene::CanvasBody>();
    bytes = bytes.saturating_add(size_of_val(ops));
    bytes = bytes.saturating_add(size_of_val(images));
    // Two text ops in one list may share one frozen paragraph. Count it once.
    let mut seen: Vec<*const crate::scene::ShapedParagraph> = Vec::new();
    for op in ops {
        match op {
            CanvasOp::Clip { path } | CanvasOp::Fill { path, .. } => {
                bytes = bytes.saturating_add(size_of_val(path.verbs.as_ref()));
            }
            CanvasOp::Text(text) => {
                bytes = bytes.saturating_add(text.utf8.len());
                if let Some(shaped) = &text.shaped {
                    let key = Arc::as_ptr(shaped);
                    if !seen.contains(&key) {
                        seen.push(key);
                        bytes = bytes.saturating_add(size_of_val(shaped.as_ref()));
                        bytes = bytes.saturating_add(size_of_val(shaped.decorations.as_ref()));
                        for run in &shaped.runs {
                            bytes = bytes.saturating_add(size_of_val(run));
                            bytes = bytes.saturating_add(size_of_val(run.glyphs.as_ref()));
                            bytes =
                                bytes.saturating_add(size_of_val(run.normalized_coords.as_ref()));
                        }
                    }
                }
            }
            CanvasOp::Image(image) => {
                bytes = bytes.saturating_add(image.source.len());
            }
            CanvasOp::Save | CanvasOp::Restore | CanvasOp::Clear { .. } => {}
        }
    }
    for image in images {
        bytes = bytes.saturating_add(image.source.len());
    }
    bytes
}
