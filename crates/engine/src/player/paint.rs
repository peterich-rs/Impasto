//! Paint construction shared by the command player. Not engine lifetime.

use std::ffi::CString;
use std::ptr;

use protocol::{ColorStop, ContentMode, Gradient, TextAlign};

use super::ffi;
use crate::Status;

pub(crate) fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// SAFETY: `paragraph` is a live Impeller paragraph.
/// `ImpellerParagraphGetLineMetrics` returns the paragraph's cached metrics
/// without a new reference (`GetC`, not `Leak`). The paragraph releases that
/// object. Releasing it here frees the cache, and the next query frees the
/// same allocation.
pub(crate) unsafe fn read_lines(
    paragraph: ffi::ImpellerParagraph,
) -> Result<Vec<crate::LineMetric>, Status> {
    let metrics = unsafe { ffi::ImpellerParagraphGetLineMetrics(paragraph) };
    if metrics.is_null() {
        return Ok(Vec::new());
    }
    let count = unsafe { ffi::ImpellerParagraphGetLineCount(paragraph) } as usize;
    let mut lines = Vec::with_capacity(count);
    for line in 0..count {
        let baseline = unsafe { ffi::ImpellerLineMetricsGetBaseline(metrics, line) };
        let ascent = unsafe { ffi::ImpellerLineMetricsGetAscent(metrics, line) };
        let height = unsafe { ffi::ImpellerLineMetricsGetHeight(metrics, line) };
        let y = (baseline - ascent) as f32;
        lines.push(crate::LineMetric {
            start: unsafe { ffi::ImpellerLineMetricsGetCodeUnitStartIndex(metrics, line) } as u32,
            end: unsafe { ffi::ImpellerLineMetricsGetCodeUnitEndIndex(metrics, line) } as u32,
            end_with_break: unsafe {
                ffi::ImpellerLineMetricsGetCodeUnitEndIndexIncludingNewline(metrics, line)
            } as u32,
            x: unsafe { ffi::ImpellerLineMetricsGetLeft(metrics, line) } as f32,
            y,
            w: unsafe { ffi::ImpellerLineMetricsGetWidth(metrics, line) } as f32,
            h: height as f32,
            baseline: baseline as f32,
            hard_break: unsafe { ffi::ImpellerLineMetricsIsHardbreak(metrics, line) },
        });
    }
    Ok(lines)
}

pub(crate) fn apply_decoration(
    style: ffi::ImpellerParagraphStyle,
    color: [u8; 4],
    underline: bool,
    line_through: bool,
) {
    let mut types = 0i32;
    if underline {
        types |= ffi::ImpellerTextDecorationType::kImpellerTextDecorationTypeUnderline as i32;
    }
    if line_through {
        types |= ffi::ImpellerTextDecorationType::kImpellerTextDecorationTypeLineThrough as i32;
    }
    if types == 0 {
        return;
    }
    #[cfg(test)]
    paint_trace::decoration(types);
    let decoration = ffi::ImpellerTextDecoration {
        types,
        color: impeller_color(color),
        style: ffi::ImpellerTextDecorationStyle::kImpellerTextDecorationStyleSolid,
        thickness_multiplier: 1.0,
    };
    unsafe { ffi::ImpellerParagraphStyleSetTextDecoration(style, &decoration) };
}

pub(crate) unsafe fn push_runs(
    style: ffi::ImpellerParagraphStyle,
    paragraph_builder: ffi::ImpellerParagraphBuilder,
    family: *const std::ffi::c_char,
    cjk: Option<&CString>,
    utf8: &str,
) {
    let runs = if cjk.is_some() {
        crate::split_scripts(utf8)
    } else {
        vec![(utf8, false)]
    };
    for (slice, use_cjk) in runs {
        if slice.is_empty() {
            continue;
        }
        let run_family = if use_cjk {
            cjk.map(|name| name.as_ptr()).unwrap_or(family)
        } else {
            family
        };
        unsafe {
            ffi::ImpellerParagraphStyleSetFontFamily(style, run_family);
            let slice_len = u32::try_from(slice.len()).unwrap_or(0);
            ffi::ImpellerParagraphBuilderPushStyle(paragraph_builder, style);
            ffi::ImpellerParagraphBuilderAddText(paragraph_builder, slice.as_ptr(), slice_len);
            ffi::ImpellerParagraphBuilderPopStyle(paragraph_builder);
        }
    }
}

pub(crate) fn paint_color(paint: ffi::ImpellerPaint, color: [u8; 4]) {
    let color = impeller_color(color);
    unsafe { ffi::ImpellerPaintSetColor(paint, &color) };
}

pub(crate) fn impeller_color(color: [u8; 4]) -> ffi::ImpellerColor {
    ffi::ImpellerColor {
        red: color[0] as f32 / 255.0,
        green: color[1] as f32 / 255.0,
        blue: color[2] as f32 / 255.0,
        alpha: color[3] as f32 / 255.0,
        color_space: ffi::ImpellerColorSpace::kImpellerColorSpaceSRGB,
    }
}

pub(crate) fn content_rects(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tex_w: f32,
    tex_h: f32,
    mode: ContentMode,
) -> Option<(ffi::ImpellerRect, ffi::ImpellerRect)> {
    match mode {
        ContentMode::Fill => Some((
            ffi::ImpellerRect {
                x: 0.0,
                y: 0.0,
                width: tex_w,
                height: tex_h,
            },
            ffi::ImpellerRect {
                x,
                y,
                width: w,
                height: h,
            },
        )),
        ContentMode::Fit => {
            if tex_w <= 0.0 || tex_h <= 0.0 {
                return None;
            }
            let scale = (w / tex_w).min(h / tex_h);
            let dw = tex_w * scale;
            let dh = tex_h * scale;
            Some((
                ffi::ImpellerRect {
                    x: 0.0,
                    y: 0.0,
                    width: tex_w,
                    height: tex_h,
                },
                ffi::ImpellerRect {
                    x: x + (w - dw) * 0.5,
                    y: y + (h - dh) * 0.5,
                    width: dw,
                    height: dh,
                },
            ))
        }
        ContentMode::Crop => {
            if tex_w <= 0.0 || tex_h <= 0.0 || w <= 0.0 || h <= 0.0 {
                return None;
            }
            let scale = (w / tex_w).max(h / tex_h);
            let src_w = w / scale;
            let src_h = h / scale;
            Some((
                ffi::ImpellerRect {
                    x: (tex_w - src_w) * 0.5,
                    y: (tex_h - src_h) * 0.5,
                    width: src_w,
                    height: src_h,
                },
                ffi::ImpellerRect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            ))
        }
        ContentMode::NineSlice { .. } => None,
    }
}

pub(crate) fn slice_rects(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tex_w: f32,
    tex_h: f32,
    mode: ContentMode,
) -> Result<Vec<(ffi::ImpellerRect, ffi::ImpellerRect)>, ()> {
    let ContentMode::NineSlice {
        left,
        top,
        right,
        bottom,
    } = mode
    else {
        return content_rects(x, y, w, h, tex_w, tex_h, mode)
            .map(|pair| vec![pair])
            .ok_or(());
    };
    if ![left, top, right, bottom, w, h, tex_w, tex_h]
        .iter()
        .all(|value| value.is_finite())
        || left < 0.0
        || top < 0.0
        || right < 0.0
        || bottom < 0.0
    {
        return Err(());
    }
    if left + right >= w || top + bottom >= h {
        return content_rects(x, y, w, h, tex_w, tex_h, ContentMode::Fill)
            .map(|pair| vec![pair])
            .ok_or(());
    }
    if left + right > tex_w || top + bottom > tex_h {
        return Err(());
    }
    let src_cols = [left, tex_w - left - right, right];
    let src_rows = [top, tex_h - top - bottom, bottom];
    let dst_cols = [left, w - left - right, right];
    let dst_rows = [top, h - top - bottom, bottom];
    let mut pairs = Vec::new();
    let mut src_y = 0.0;
    let mut dst_y = y;
    for row in 0..3 {
        let mut src_x = 0.0;
        let mut dst_x = x;
        for col in 0..3 {
            let sw = src_cols[col];
            let sh = src_rows[row];
            let dw = dst_cols[col];
            let dh = dst_rows[row];
            if sw > 0.0 && sh > 0.0 && dw > 0.0 && dh > 0.0 {
                if src_x < 0.0
                    || src_y < 0.0
                    || src_x + sw > tex_w + 0.01
                    || src_y + sh > tex_h + 0.01
                {
                    return Err(());
                }
                pairs.push((
                    ffi::ImpellerRect {
                        x: src_x,
                        y: src_y,
                        width: sw,
                        height: sh,
                    },
                    ffi::ImpellerRect {
                        x: dst_x,
                        y: dst_y,
                        width: dw,
                        height: dh,
                    },
                ));
            }
            src_x += sw;
            dst_x += dw;
        }
        src_y += src_rows[row];
        dst_y += dst_rows[row];
    }
    Ok(pairs)
}

pub(crate) fn gradient_solid(gradient: Option<&Gradient>) -> Option<[u8; 4]> {
    let Some(gradient) = gradient else {
        return None;
    };
    match prepared_gradient(gradient) {
        PreparedGradient::Solid(color) => Some(color),
        PreparedGradient::Paint { .. } => None,
        PreparedGradient::Invalid => None,
    }
}

pub(crate) enum PreparedGradient {
    Invalid,
    Solid([u8; 4]),
    Paint {
        kind: GradientKind,
        stops: Vec<ColorStop>,
    },
}

pub(crate) enum GradientKind {
    Linear {
        start: [f32; 2],
        end: [f32; 2],
    },
    Radial {
        center: [f32; 2],
        radius: f32,
    },
    Sweep {
        center: [f32; 2],
        start_angle_rad: f32,
        end_angle_rad: f32,
    },
}

pub(crate) fn prepared_gradient(gradient: &Gradient) -> PreparedGradient {
    match gradient {
        Gradient::Linear { start, end, stops } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if start[0] == end[0] && start[1] == end[1] {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Linear {
                    start: *start,
                    end: *end,
                },
                stops,
            }
        }
        Gradient::Radial {
            center,
            radius,
            stops,
        } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if !radius.is_finite() || *radius <= 0.0 {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Radial {
                    center: *center,
                    radius: *radius,
                },
                stops,
            }
        }
        Gradient::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
            stops,
        } => {
            let Some(stops) = normalize_stops(stops) else {
                return PreparedGradient::Invalid;
            };
            if !start_angle_rad.is_finite() || !end_angle_rad.is_finite() {
                return PreparedGradient::Invalid;
            }
            if start_angle_rad == end_angle_rad {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0; 4]),
                );
            }
            PreparedGradient::Paint {
                kind: GradientKind::Sweep {
                    center: *center,
                    start_angle_rad: *start_angle_rad,
                    end_angle_rad: *end_angle_rad,
                },
                stops,
            }
        }
    }
}

pub(crate) fn normalize_stops(stops: &[ColorStop]) -> Option<Vec<ColorStop>> {
    if stops.len() < 2 {
        return None;
    }
    if stops
        .iter()
        .any(|stop| !stop.offset.is_finite() || stop.offset < 0.0 || stop.offset > 1.0)
    {
        return None;
    }
    let mut indexed: Vec<(usize, ColorStop)> = stops.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.offset
            .partial_cmp(&b.1.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    Some(indexed.into_iter().map(|(_, stop)| stop).collect())
}

pub(crate) fn color_source(
    gradient: Option<&Gradient>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Option<ffi::ImpellerColorSource> {
    let PreparedGradient::Paint { kind, stops } = prepared_gradient(gradient?) else {
        return None;
    };
    let colors: Vec<ffi::ImpellerColor> = stops
        .iter()
        .map(|stop| impeller_color(stop.color))
        .collect();
    let offsets: Vec<f32> = stops.iter().map(|stop| stop.offset).collect();
    let count = u32::try_from(stops.len()).unwrap_or(0);
    match kind {
        GradientKind::Linear { start, end } => {
            let start = ffi::ImpellerPoint {
                x: x + start[0] * w,
                y: y + start[1] * h,
            };
            let end = ffi::ImpellerPoint {
                x: x + end[0] * w,
                y: y + end[1] * h,
            };
            let source = unsafe {
                ffi::ImpellerColorSourceCreateLinearGradientNew(
                    &start,
                    &end,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
        GradientKind::Radial { center, radius } => {
            let center = ffi::ImpellerPoint {
                x: x + center[0] * w,
                y: y + center[1] * h,
            };
            let scene_radius = radius * w.min(h);
            let source = unsafe {
                ffi::ImpellerColorSourceCreateRadialGradientNew(
                    &center,
                    scene_radius,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
        GradientKind::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
        } => {
            let center = ffi::ImpellerPoint {
                x: x + center[0] * w,
                y: y + center[1] * h,
            };
            let start = start_angle_rad * (180.0 / std::f32::consts::PI);
            let end = end_angle_rad * (180.0 / std::f32::consts::PI);
            let source = unsafe {
                ffi::ImpellerColorSourceCreateSweepGradientNew(
                    &center,
                    start,
                    end,
                    count,
                    colors.as_ptr(),
                    offsets.as_ptr(),
                    ffi::ImpellerTileMode::kImpellerTileModeClamp,
                    ptr::null(),
                )
            };
            if source.is_null() { None } else { Some(source) }
        }
    }
}

pub(crate) fn rect_path(x: f32, y: f32, w: f32, h: f32, corner_radius: f32) -> ffi::ImpellerPath {
    let builder = unsafe { ffi::ImpellerPathBuilderNew() };
    if builder.is_null() {
        return ptr::null_mut();
    }
    let rect = ffi::ImpellerRect {
        x,
        y,
        width: w,
        height: h,
    };
    let radius = clamped_radius(corner_radius, w, h);
    unsafe {
        if radius > 0.0 {
            let radii = uniform_radii(radius);
            ffi::ImpellerPathBuilderAddRoundedRect(builder, &rect, &radii);
        } else {
            ffi::ImpellerPathBuilderAddRect(builder, &rect);
        }
        let path = ffi::ImpellerPathBuilderTakePathNew(
            builder,
            ffi::ImpellerFillType::kImpellerFillTypeNonZero,
        );
        ffi::ImpellerPathBuilderRelease(builder);
        path
    }
}

pub(crate) fn impeller_weight(weight: u16) -> ffi::ImpellerFontWeight::Type {
    match weight {
        100 => ffi::ImpellerFontWeight::kImpellerFontWeight100,
        200 => ffi::ImpellerFontWeight::kImpellerFontWeight200,
        300 => ffi::ImpellerFontWeight::kImpellerFontWeight300,
        500 => ffi::ImpellerFontWeight::kImpellerFontWeight500,
        600 => ffi::ImpellerFontWeight::kImpellerFontWeight600,
        700 => ffi::ImpellerFontWeight::kImpellerFontWeight700,
        800 => ffi::ImpellerFontWeight::kImpellerFontWeight800,
        900 => ffi::ImpellerFontWeight::kImpellerFontWeight900,
        _ => ffi::ImpellerFontWeight::kImpellerFontWeight400,
    }
}

pub(crate) fn impeller_align(align: TextAlign) -> ffi::ImpellerTextAlignment::Type {
    match align {
        TextAlign::Left => ffi::ImpellerTextAlignment::kImpellerTextAlignmentLeft,
        TextAlign::Center => ffi::ImpellerTextAlignment::kImpellerTextAlignmentCenter,
        TextAlign::Right => ffi::ImpellerTextAlignment::kImpellerTextAlignmentRight,
    }
}

pub(crate) fn clamped_radius(radius: f32, width: f32, height: f32) -> f32 {
    if !radius.is_finite() || radius <= 0.0 || width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    radius.min(width.min(height) * 0.5)
}

/// Arguments passed into Impeller for units that must stay in scene points.
#[cfg(test)]
pub(crate) mod paint_trace {
    use std::cell::RefCell;

    #[derive(Clone, Debug, Default)]
    pub struct Trace {
        pub stroke_widths: Vec<f32>,
        pub blur_sigmas: Vec<(f32, f32)>,
        pub shadows: Vec<(f32, f32)>,
        pub decorations: Vec<i32>,
    }

    thread_local! {
        static TRACE: RefCell<Trace> = RefCell::new(Trace::default());
    }

    pub fn clear() {
        TRACE.with(|trace| *trace.borrow_mut() = Trace::default());
    }

    pub fn take() -> Trace {
        TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
    }

    pub fn stroke(width: f32) {
        TRACE.with(|trace| trace.borrow_mut().stroke_widths.push(width));
    }

    pub fn blur(sigma_x: f32, sigma_y: f32) {
        TRACE.with(|trace| trace.borrow_mut().blur_sigmas.push((sigma_x, sigma_y)));
    }

    pub fn shadow(elevation: f32, device_pixel_ratio: f32) {
        TRACE.with(|trace| {
            trace
                .borrow_mut()
                .shadows
                .push((elevation, device_pixel_ratio));
        });
    }

    pub fn decoration(types: i32) {
        TRACE.with(|trace| trace.borrow_mut().decorations.push(types));
    }
}

pub(crate) fn uniform_radii(radius: f32) -> ffi::ImpellerRoundingRadii {
    let corner = ffi::ImpellerPoint {
        x: radius,
        y: radius,
    };
    ffi::ImpellerRoundingRadii {
        top_left: corner,
        bottom_left: corner,
        top_right: corner,
        bottom_right: corner,
    }
}
