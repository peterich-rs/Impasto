// Analytic rectangles and tessellated paths in linear premultiplied color.
@group(0) @binding(1) var<uniform> stops: array<vec4<f32>, 512>;
@group(1) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(1) @binding(1) var clip_mask_sampler: sampler;

struct RectVertex {
    @location(1) rect: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) color: vec4<f32>,
    @location(4) stroke_color: vec4<f32>,
    @location(5) gradient_a: vec4<f32>,
    @location(6) gradient_b: vec4<f32>,
    @location(7) shadow: vec4<f32>,
    @location(8) extra: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local_coverage: vec3<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) shape: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) stroke_color: vec4<f32>,
    @location(5) @interpolate(flat) gradient_a: vec4<f32>,
    @location(6) @interpolate(flat) gradient_b: vec4<f32>,
    @location(7) @interpolate(flat) shadow: vec4<f32>,
};

fn vertex_out(data: RectVertex, p: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(p.xy, 0.0, 1.0);
    out.local_coverage = p;
    out.rect = data.rect;
    out.shape = data.shape;
    out.color = data.color;
    out.stroke_color = data.stroke_color;
    out.gradient_a = data.gradient_a;
    out.gradient_b = data.gradient_b;
    out.shadow = data.shadow;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: RectVertex) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let margin = (data.shape.y * 0.5 + 1.0 / max(view.info.x, 0.001)) * data.extra.x;
    let scene = data.rect.xy - vec2(margin) + corners[vertex] * (data.rect.zw + vec2(2.0 * margin));
    return vertex_out(data, vec3<f32>(scene, 1.0));
}

@vertex
fn vs_mesh(@location(0) vertex: vec3<f32>, data: RectVertex) -> VertexOut {
    return vertex_out(data, vertex);
}

// Stops are stored in pairs: even = premultiplied color, odd = (offset, 0, 0, 0).
fn sample_gradient(t: f32, first: u32, count: u32) -> vec4<f32> {
    if (count == 0u) {
        return vec4<f32>(0.0);
    }
    let clamped = clamp(t, 0.0, 1.0);
    let last = count - 1u;
    var i = 0u;
    loop {
        if (i >= last || clamped <= stops[first + (i + 1u) * 2u + 1u].x) {
            break;
        }
        i = i + 1u;
    }
    let c0 = stops[first + i * 2u];
    let o0 = stops[first + i * 2u + 1u].x;
    let j = min(i + 1u, last);
    let c1 = stops[first + j * 2u];
    let o1 = stops[first + j * 2u + 1u].x;
    let f = clamp((clamped - o0) / max(o1 - o0, 1e-6), 0.0, 1.0);
    return mix(c0, c1, f);
}

// Solve |p - lerp(c0, c1, t)| = lerp(r0, r1, t), using the largest
// root with a positive radius. A negative result outside the cone is not paint.
fn conical_position(p: vec2<f32>, centers: vec4<f32>, radii: vec2<f32>) -> vec2<f32> {
    let d = centers.zw - centers.xy;
    let q = p - centers.xy;
    let dr = radii.y - radii.x;
    let a = dot(d, d) - dr * dr;
    let b = -2.0 * (dot(q, d) + radii.x * dr);
    let c = dot(q, q) - radii.x * radii.x;
    if (abs(a) < 1e-6) {
        if (abs(b) < 1e-6) { return vec2<f32>(0.0, 0.0); }
        let t = -c / b;
        return vec2<f32>(t, select(0.0, 1.0, radii.x + t * dr >= 0.0));
    }
    let discriminant = b * b - 4.0 * a * c;
    if (discriminant < 0.0) { return vec2<f32>(0.0, 0.0); }
    let root = sqrt(discriminant);
    let t0 = (-b - root) / (2.0 * a);
    let t1 = (-b + root) / (2.0 * a);
    let farther = max(t0, t1);
    let t = select(min(t0, t1), farther, radii.x + farther * dr >= 0.0);
    return vec2<f32>(t, select(0.0, 1.0, radii.x + t * dr >= 0.0));
}

fn gradient_color(data: VertexOut, p: vec2<f32>) -> vec4<f32> {
    let kind = u32(data.gradient_b.x);
    let first = u32(data.gradient_b.y);
    let count = u32(data.gradient_b.z);
    var t = 0.0;
    if (kind == 1u) {
        let center = data.gradient_a.xy;
        let radius = data.gradient_a.z;
        t = length(p - center) / max(radius, 1e-6);
    } else if (kind == 3u) {
        let conical = conical_position(p, data.gradient_a, data.shadow.xy);
        return sample_gradient(conical.x, first, count) * conical.y;
    } else if (kind == 2u) {
        let center = data.gradient_a.xy;
        let start = data.gradient_a.z;
        let end = data.gradient_a.w;
        let angle = atan2(p.y - center.y, p.x - center.x) - start;
        let wrapped = angle - 6.28318530718 * floor(angle / 6.28318530718);
        t = wrapped / max(end - start, 1e-6);
    } else {
        let start = data.gradient_a.xy;
        let end = data.gradient_a.zw;
        let d = end - start;
        t = dot(p - start, d) / max(dot(d, d), 1e-6);
    }
    return sample_gradient(t, first, count);
}

fn erf(x: f32) -> f32 {
    // Abramowitz-Stegun 7.1.26, max error 1.5e-7.
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

// Analytic rounded-box shadow: the Gaussian integral of the box's distance
// field (Evan Wallace, "Fast Rounded-Rectangle Shadows"). One quad, no blur pass.
fn shadow_alpha(data: VertexOut, p: vec2<f32>) -> f32 {
    // rect is the rect grown by the shadow margin. Recover the spread box.
    let spread = data.shadow.z;
    let margin = data.shadow.w * 3.0;
    let half = data.rect.zw * 0.5 - vec2<f32>(margin);
    let center = data.rect.xy + data.rect.zw * 0.5;
    let sigma = max(data.shadow.w, 0.0);
    let r = data.shape.x + spread;
    let q = abs(p - center) - half + vec2<f32>(r);
    let outside = length(max(q, vec2<f32>(0.0)));
    let inside = min(max(q.x, q.y), 0.0) - r;
    let dist = outside + inside;
    if (sigma < 0.3) {
        return select(0.0, 1.0, dist <= 0.0);
    }
    return 0.5 + 0.5 * erf(-dist * 0.70710678 / sigma);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let aa = max(length(dpdx(in.local_coverage.xy)) + length(dpdy(in.local_coverage.xy)), 1e-6) * 0.5;
    let p = in.local_coverage.xy;
    let kind = u32(data.shape.z);
    var color = vec4<f32>(0.0);

    if (kind == 3u) {
        color = data.color * shadow_alpha(data, p);
    } else if (kind == 4u) {
        // Clip mask target: write coverage of the rounded rect into r and a.
        let half = data.rect.zw * 0.5;
        let center = data.rect.xy + half;
        let d = sd_rounded_box(p - center, half, data.shape.x);
        let a = clamp(0.5 - d / aa, 0.0, 1.0);
        color = vec4<f32>(a, 0.0, 0.0, a);
    } else if (kind == 5u) {
        // Mesh: lyon wrote per-vertex coverage; the brush is the color.
        color = data.color;
    } else {
        let half = data.rect.zw * 0.5;
        let center = data.rect.xy + half;
        let d = sd_rounded_box(p - center, half, data.shape.x);
        if (kind == 0u || kind == 2u) {
            let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
            var brush = data.color;
            if ((u32(data.shape.w) & 1u) != 0u) {
                brush = gradient_color(data, p);
            }
            color = brush * coverage;
        }
        if (kind == 1u || kind == 2u) {
            let w = max(data.shape.y, 0.0);
            let sd = abs(d) - w * 0.5;
            let sc = clamp(0.5 - sd / aa, 0.0, 1.0);
            let src = data.stroke_color * sc;
            color = src + color * (1.0 - src.a);
        }
    }

    color = color * clip_coverage(in.position.xy) * data.gradient_b.w;
    return color;
}

@fragment
fn fs_mesh(in: VertexOut) -> @location(0) vec4<f32> {
    var color = in.color;
    if ((u32(in.shape.w) & 1u) != 0u) {
        color = gradient_color(in, in.local_coverage.xy) * in.gradient_b.w;
    }
    return color * in.local_coverage.z * clip_coverage(in.position.xy);
}
