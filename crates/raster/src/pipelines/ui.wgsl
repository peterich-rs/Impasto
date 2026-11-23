// Analytic rectangles and tessellated paths in linear premultiplied color.
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

fn paint_gradient(data: VertexOut, p: vec2<f32>) -> vec4<f32> {
    let indices = vec3<u32>(data.gradient_b.xyz);
    return gradient_color(p, indices.x, indices.y, indices.z, data.gradient_a, data.shadow.xy);
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
                brush = paint_gradient(data, p);
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
        color = paint_gradient(in, in.local_coverage.xy) * in.gradient_b.w;
    }
    return color * in.local_coverage.z * clip_coverage(in.position.xy);
}
