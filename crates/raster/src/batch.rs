//! Draw batches. The compiler turns commands into ordered batches of
//! instance records; the compositor binds textures and encodes them.
//!
//! A batch is one pipeline, one texture set, and one clip state. Adjacent
//! draws with the same key merge so a row of rectangles is one draw call.

use engine::scene::{BlendMode, ColorStop, ContentMode, FillRule, Gradient, PathVerb};

/// Compiled rectangle paint in scene points.
#[derive(Clone, Debug, PartialEq)]
pub struct RectInstance {
    pub rect: [f32; 4],
    pub radius: f32,
    pub stroke_width: f32,
    pub kind: u32,
    pub flags: u32,
    pub color: [f32; 4],
    pub stroke_color: [f32; 4],
    pub gradient_a: [f32; 4],
    pub gradient_b: [f32; 4],
    pub shadow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectVertex {
    pub rect: [f32; 4],
    pub shape: [f32; 4],
    pub color: [f32; 4],
    pub stroke_color: [f32; 4],
    pub gradient_a: [f32; 4],
    pub gradient_b: [f32; 4],
    pub shadow: [f32; 4],
    pub extra: [f32; 4],
}

impl RectInstance {
    pub fn vertex(&self, first: f32) -> RectVertex {
        let mut gradient_b = self.gradient_b;
        gradient_b[1] = first;
        RectVertex {
            rect: self.rect,
            shape: [
                self.radius,
                self.stroke_width,
                self.kind as f32,
                self.flags as f32,
            ],
            color: self.color,
            stroke_color: self.stroke_color,
            gradient_a: self.gradient_a,
            gradient_b,
            shadow: self.shadow,
            extra: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

pub const RECT_FILL: u32 = 0;
pub const RECT_STROKE: u32 = 1;
pub const RECT_FILL_STROKE: u32 = 2;
pub const RECT_SHADOW: u32 = 3;
pub const RECT_GRADIENT: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphVertex {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub meta: [f32; 4],
    pub gradient_a: [f32; 4],
    pub gradient_b: [f32; 4],
}

pub const GLYPH_COLOR: u32 = 1;
pub const GLYPH_GRADIENT: u32 = 2;
pub const GLYPH_DECORATION: u32 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct ImageInstance {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageVertex {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub meta: [f32; 4],
}

impl ImageInstance {
    pub fn vertex(&self, premultiplied_srgb: bool) -> ImageVertex {
        let flags = self.flags
            | if premultiplied_srgb {
                IMAGE_PREMULTIPLIED_SRGB
            } else {
                0
            };
        ImageVertex {
            rect: self.rect,
            uv: self.uv,
            color: self.color,
            meta: [flags as f32, 0.0, 0.0, 0.0],
        }
    }
}

pub const IMAGE_NEAREST: u32 = 1;
pub const IMAGE_PREMULTIPLIED_SRGB: u32 = 2;

#[derive(Debug, PartialEq)]
pub struct GpuMeshBuffer {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
}

/// A tessellated path's range in its picture's resident buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub buffer: std::sync::Arc<std::sync::OnceLock<GpuMeshBuffer>>,
    pub indices: std::ops::Range<u32>,
    pub bounds: [f32; 4],
    pub brush: Brush,
    pub composite: engine::scene::CanvasComposite,
}

/// Brush of a mesh or decoration.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    Solid([f32; 4]),
    Gradient(PackedGradient),
}

impl Brush {
    /// The solid color, or white for a gradient (the shader samples it).
    pub fn solid(&self) -> [f32; 4] {
        match self {
            Self::Solid(color) => *color,
            Self::Gradient(_) => [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// Gradient geometry shared by every pipeline. `stops` are packed pairs.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedGradient {
    pub kind: u32,
    /// Linear: start xy, end xy. Radial: center xy, radius, start radius.
    /// Sweep: center xy, start angle, end angle.
    pub a: [f32; 4],
    pub radial_radii: [f32; 2],
    /// Stop pairs: premultiplied color, `(offset, 0, 0, 0)`.
    pub stops: Vec<[f32; 4]>,
}

/// A gradient with its unit-square geometry mapped into scene points.
pub fn pack_gradient(
    gradient: &Gradient,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: u8,
) -> Option<PackedGradient> {
    let (kind, a, stops) = match gradient {
        Gradient::Linear { start, end, stops } => (
            0,
            [
                x + start[0] * w,
                y + start[1] * h,
                x + end[0] * w,
                y + end[1] * h,
            ],
            stops,
        ),
        Gradient::Radial {
            center,
            radius,
            stops,
        } => (
            1,
            [x + center[0] * w, y + center[1] * h, radius * w.min(h), 0.0],
            stops,
        ),
        Gradient::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
            stops,
        } => (
            2,
            [
                x + center[0] * w,
                y + center[1] * h,
                *start_angle_rad,
                *end_angle_rad,
            ],
            stops,
        ),
    };
    Some(PackedGradient {
        kind,
        a,
        radial_radii: [0.0; 2],
        stops: pack_stops(stops, alpha)?,
    })
}

/// A canvas gradient already in canvas space.
pub fn pack_canvas_gradient(
    brush: &engine::scene::CanvasBrush,
    alpha: u8,
) -> Option<PackedGradient> {
    match brush {
        engine::scene::CanvasBrush::Color(color) => Some(PackedGradient {
            kind: 0,
            a: [0.0; 4],
            radial_radii: [0.0; 2],
            stops: vec![
                crate::color::linear_premultiplied(*color, alpha)?,
                [0.0; 4],
                crate::color::linear_premultiplied(*color, alpha)?,
                [1.0, 0.0, 0.0, 0.0],
            ],
        }),
        engine::scene::CanvasBrush::Linear { start, end, stops } => Some(PackedGradient {
            kind: 0,
            a: [start[0], start[1], end[0], end[1]],
            radial_radii: [0.0; 2],
            stops: pack_stops(stops, alpha)?,
        }),
        engine::scene::CanvasBrush::Radial {
            start,
            start_radius,
            end,
            end_radius,
            stops,
        } => Some(PackedGradient {
            kind: 3,
            a: [start[0], start[1], end[0], end[1]],
            radial_radii: [*start_radius, *end_radius],
            stops: pack_stops(stops, alpha)?,
        }),
        engine::scene::CanvasBrush::Conic {
            center,
            angle,
            stops,
        } => Some(PackedGradient {
            kind: 2,
            a: [center[0], center[1], *angle, *angle + std::f32::consts::TAU],
            radial_radii: [0.0; 2],
            stops: pack_stops(stops, alpha)?,
        }),
    }
}

/// Stop pairs as the shaders read them.
pub fn pack_stops(stops: &[ColorStop], alpha: u8) -> Option<Vec<[f32; 4]>> {
    if stops.len() < 2 || stops.len() > crate::pipelines::MAX_STOPS {
        return None;
    }
    let mut packed = Vec::with_capacity(stops.len() * 2);
    for stop in stops {
        if !stop.offset.is_finite() {
            return None;
        }
        packed.push(crate::color::linear_premultiplied(stop.color, alpha)?);
        packed.push([stop.offset, 0.0, 0.0, 0.0]);
    }
    Some(packed)
}

type ImageSlice = ([f32; 4], [f32; 4]);

/// Maps a content mode to destination rectangles in scene points and source
/// rectangles in normalized UV. Nine-slice yields up to nine pairs.
pub fn image_slices(
    mode: ContentMode,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    tex_w: f32,
    tex_h: f32,
) -> Option<Vec<ImageSlice>> {
    let fill = ([x, y, w, h], [0.0, 0.0, 1.0, 1.0]);
    match mode {
        ContentMode::Fill => Some(vec![fill]),
        ContentMode::Fit | ContentMode::Crop => {
            if !(tex_w > 0.0 && tex_h > 0.0 && w > 0.0 && h > 0.0) {
                return Some(Vec::new());
            }
            let scale_w = w / tex_w;
            let scale_h = h / tex_h;
            let scale = if mode == ContentMode::Fit {
                scale_w.min(scale_h)
            } else {
                scale_w.max(scale_h)
            };
            let (dw, dh) = (tex_w * scale, tex_h * scale);
            let (dx, dy) = ((w - dw) * 0.5, (h - dh) * 0.5);
            if mode == ContentMode::Fit {
                return Some(vec![([x + dx, y + dy, dw, dh], [0.0, 0.0, 1.0, 1.0])]);
            }
            // Clip the destination to the box and map the visible part back
            // to source UV so the sampler never reads outside the image.
            let left = (-dx / dw).max(0.0);
            let top = (-dy / dh).max(0.0);
            let right = ((w - dx) / dw).min(1.0);
            let bottom = ((h - dy) / dh).min(1.0);
            Some(vec![([x, y, w, h], [left, top, right, bottom])])
        }
        ContentMode::NineSlice {
            left,
            top,
            right,
            bottom,
        } => {
            if ![left, top, right, bottom, w, h, tex_w, tex_h]
                .into_iter()
                .all(|value| value.is_finite() && value >= 0.0)
            {
                return None;
            }
            // Preserve Fill for destinations too small to hold the fixed edges.
            if left + right >= w || top + bottom >= h {
                return Some(vec![fill]);
            }
            if tex_w <= 0.0 || tex_h <= 0.0 || left + right > tex_w || top + bottom > tex_h {
                return None;
            }
            let sl = left / tex_w;
            let st = top / tex_h;
            let sr = (tex_w - right) / tex_w;
            let sb = (tex_h - bottom) / tex_h;
            let xs = [
                (x, left, 0.0, sl),
                (x + left, w - left - right, sl, sr),
                (x + w - right, right, sr, 1.0),
            ];
            let ys = [
                (y, top, 0.0, st),
                (y + top, h - top - bottom, st, sb),
                (y + h - bottom, bottom, sb, 1.0),
            ];
            let mut out = Vec::with_capacity(9);
            for (dy, dh, v0, v1) in ys {
                for (dx, dw, u0, u1) in xs {
                    if dw > 0.0 && dh > 0.0 && u1 > u0 && v1 > v0 {
                        out.push(([dx, dy, dw, dh], [u0, v0, u1, v1]));
                    }
                }
            }
            Some(out)
        }
    }
}

/// Layer blend mapped to the compositor's blend mode index.
pub fn blend_mode(mode: BlendMode) -> u32 {
    match mode {
        BlendMode::SrcOver => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
    }
}

pub fn canvas_composite(mode: engine::scene::CanvasComposite) -> u32 {
    use engine::scene::CanvasComposite as C;
    match mode {
        C::SourceOver => 0,
        C::Multiply => 1,
        C::Screen => 2,
        C::DestinationOver => 3,
        C::DestinationOut => 4,
        C::Lighter => 5,
        C::Xor => 6,
    }
}

/// Build a lyon path from recorded verbs.
pub fn tessellation_path(verbs: &[PathVerb]) -> lyon_path::Path {
    let mut builder = lyon_path::Path::builder();
    let point = |x: f32, y: f32| lyon_geom::Point::new(x, y);
    let mut open = false;
    for verb in verbs {
        match *verb {
            PathVerb::Move(x, y) => {
                if open {
                    builder.end(false);
                }
                builder.begin(point(x, y));
                open = true;
            }
            PathVerb::Line(x, y) => {
                if !open {
                    builder.begin(point(x, y));
                    open = true;
                } else {
                    builder.line_to(point(x, y));
                }
            }
            PathVerb::Quad(cx, cy, x, y) => {
                if !open {
                    builder.begin(point(cx, cy));
                    open = true;
                }
                builder.quadratic_bezier_to(point(cx, cy), point(x, y));
            }
            PathVerb::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                if !open {
                    builder.begin(point(c1x, c1y));
                    open = true;
                }
                builder.cubic_bezier_to(point(c1x, c1y), point(c2x, c2y), point(x, y));
            }
            PathVerb::Close => {
                if open {
                    builder.end(true);
                    open = false;
                }
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

pub fn lyon_fill_rule(rule: FillRule) -> lyon_tessellation::FillRule {
    match rule {
        FillRule::NonZero => lyon_tessellation::FillRule::NonZero,
        FillRule::EvenOdd => lyon_tessellation::FillRule::EvenOdd,
    }
}
