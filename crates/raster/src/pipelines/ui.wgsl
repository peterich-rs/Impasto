// Analytic rectangles and tessellated paths in linear premultiplied color.
override simple_paint: bool = false;
override two_dimensional_shadow: bool = false;
override output_dither: bool = false;
@group(1) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(1) @binding(1) var clip_mask_sampler: sampler;

struct RectVertex {
    @location(1) rect: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) opacity: f32,
    @location(5) paint_index: u32,
    @location(6) depth: f32,
    @location(7) curve: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local_coverage: vec3<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) shape: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) opacity: f32,
    @location(5) @interpolate(flat) paint_index: u32,
    @location(6) @interpolate(flat) curve: f32,
};

fn vertex_out(data: RectVertex, p: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(p.xy, 0.0, 1.0);
    out.local_coverage = p;
    out.rect = data.rect;
    out.shape = data.shape;
    out.color = data.color;
    out.opacity = data.opacity;
    out.paint_index = data.paint_index;
    out.curve = data.curve;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: RectVertex) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let margin = (data.shape.y * 0.5 + 1.0 / max(view.info.x, 0.001));
    let scene = data.rect.xy - vec2(margin) + corners[vertex] * (data.rect.zw + vec2(2.0 * margin));
    return vertex_out(data, vec3<f32>(scene, 1.0));
}

@vertex
fn vs_mesh(@location(0) vertex: vec3<f32>, data: RectVertex) -> VertexOut {
    return vertex_out(data, vertex);
}

fn erf(x: f32) -> f32 {
    // Abramowitz-Stegun 7.1.26, max error 1.5e-7.
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

fn box_distance(data: VertexOut, p: vec2<f32>) -> f32 {
    let half = data.rect.zw * 0.5;
    let centered = p - (data.rect.xy + half);
    if (data.curve >= 2.0) {
        return sd_shaped_box(centered, half, load_paint(data.paint_index).corners, data.curve);
    }
    return sd_rounded_box(centered, half, data.shape.x);
}

fn corner_cut(distance: f32, r: f32, curve: f32) -> f32 {
    if (r <= 0.0 || distance >= r) { return 0.0; }
    let y = clamp(1.0 - distance / r, 0.0, 1.0);
    return r * (1.0 - pow(max(0.0, 1.0 - pow(y, curve)), 1.0 / curve));
}

// Integrate the horizontal Gaussian analytically over each row of the
// shape and integrate the vertical Gaussian over its finite halo.
fn shadow_alpha(data: VertexOut, p: vec2<f32>, extra: PaintExtra) -> f32 {
    let sigma = max(extra.shadow.w, 0.0);
    let half = data.rect.zw * 0.5 - vec2(3.0 * sigma);
    if (any(half <= vec2(0.0))) { return 0.0; }
    let center = data.rect.xy + data.rect.zw * 0.5;
    let q = p - center;
    let r = clamp(data.shape.x + extra.shadow.z, 0.0, min(half.x, half.y));
    let radii = select(vec4(r), extra.corners, data.curve >= 2.0);
    let curve = max(data.curve, 2.0);
    if !two_dimensional_shadow {
        var distance = sd_rounded_box(q, half, r);
        if (data.curve >= 2.0) {
            distance = sd_shaped_box(q, half, radii, curve);
        }
        if (sigma < 0.3) {
            return select(0.0, 1.0, distance <= 0.0);
        }
        return 0.5 + 0.5 * erf(-distance * 0.70710678 / sigma);
    }
    if (sigma < 0.3) {
        return select(0.0, 1.0, sd_shaped_box(q, half, radii, curve) <= 0.0);
    }
    let inv = 0.70710678118 / sigma;
    let low = max(-half.y, q.y - 3.0 * sigma);
    let high = min(half.y, q.y + 3.0 * sigma);
    if (high <= low) { return 0.0; }
    // The quadrature is local in Y, including for a tall rectangle.
    let step = (high - low) / 8.0;
    var integral = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let y = low + (f32(i) + 0.5) * step;
        let top = y + half.y;
        let bottom = half.y - y;
        let left = -half.x + max(corner_cut(top, radii.x, curve), corner_cut(bottom, radii.w, curve));
        let right = half.x - max(corner_cut(top, radii.y, curve), corner_cut(bottom, radii.z, curve));
        let horizontal = 0.5 * (erf((right - q.x) * inv) - erf((left - q.x) * inv));
        let vertical = (y - q.y) * inv;
        integral += horizontal * exp(-vertical * vertical);
    }
    return clamp(integral * step * 0.3989422804 / sigma, 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let aa = max(length(dpdx(in.local_coverage.xy)) + length(dpdy(in.local_coverage.xy)), 1e-6) * 0.5;
    let p = in.local_coverage.xy;
    if simple_paint {
        let half = data.rect.zw * 0.5;
        let d = sd_rounded_box(p - (data.rect.xy + half), half, data.shape.x);
        return data.color * clamp(0.5 - d / aa, 0.0, 1.0) * data.opacity;
    }
    let kind = u32(data.shape.z);
    var color = vec4<f32>(0.0);

    if (kind == 3u) {
        color = data.color * shadow_alpha(data, p, load_paint(data.paint_index));
    } else if (kind == 4u) {
        // Clip mask target: write coverage of the rounded rect into r and a.
        let d = box_distance(data, p);
        let a = clamp(0.5 - d / aa, 0.0, 1.0);
        color = vec4<f32>(a, 0.0, 0.0, a);
    } else if (kind == 5u) {
        // Mesh: lyon wrote per-vertex coverage; the brush is the color.
        color = data.color;
    } else {
        let d = box_distance(data, p);
        if (kind == 0u || kind == 2u) {
            let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
            var brush = data.color;
            if ((u32(data.shape.w) & 1u) != 0u) {
                brush = paint_gradient(load_paint(data.paint_index), p);
            }
            color = brush * coverage;
        }
        if (kind == 1u || kind == 2u) {
            let w = max(data.shape.y, 0.0);
            let sd = abs(d) - w * 0.5;
            let sc = clamp(0.5 - sd / aa, 0.0, 1.0);
            let src = load_paint(data.paint_index).stroke * sc;
            color = src + color * (1.0 - src.a);
        }
    }

    color = color * clip_coverage(in.position.xy) * data.opacity;
    if output_dither && view.info.w > 0.0 && color.a >= 1.0 && (u32(data.shape.w) & 1u) != 0u {
        return vec4(to_linear(dither_srgb(color, in.position.xy).rgb), color.a);
    }
    return color;
}

@fragment
fn fs_mesh(in: VertexOut) -> @location(0) vec4<f32> {
    var color = in.color;
    if ((u32(in.shape.w) & 1u) != 0u) {
        color = paint_gradient(load_paint(in.paint_index), in.local_coverage.xy) * in.opacity;
    }
    return color * in.local_coverage.z * clip_coverage(in.position.xy);
}


struct DepthOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs_depth_rect(@builtin(vertex_index) vertex: u32, data: RectVertex) -> DepthOut {
    let corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
        vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let scene = data.rect.xy + corners[vertex] * data.rect.zw;
    return DepthOut(view.transform * vec4(scene, data.depth, 1.0), data.color);
}

@fragment
fn fs_depth_rect(in: DepthOut) -> @location(0) vec4<f32> {
    return in.color;
}


@vertex
fn vs_stencil(@location(0) vertex: vec3<f32>) -> @builtin(position) vec4<f32> {
    return view.transform * vec4(vertex.xy, 0.0, 1.0);
}

@fragment
fn fs_stencil() -> @location(0) vec4<f32> {
    return vec4(0.0);
}
