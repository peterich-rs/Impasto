// Glyphs and decoration bars.
@group(0) @binding(1) var<uniform> stops: array<vec4<f32>, 512>;
@group(1) @binding(0) var mask_atlas: texture_2d_array<f32>;
@group(1) @binding(1) var color_atlas: texture_2d_array<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;
@group(2) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(2) @binding(1) var clip_mask_sampler: sampler;

struct GlyphVertex {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) info: vec4<f32>,
    @location(4) gradient_a: vec4<f32>,
    @location(5) gradient_b: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) info: vec4<f32>,
    @location(4) @interpolate(flat) gradient_a: vec4<f32>,
    @location(5) @interpolate(flat) gradient_b: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: GlyphVertex) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let unit = corners[vertex];
    let scene = data.rect.xy + unit * data.rect.zw;
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.uv = mix(data.uv.xy, data.uv.zw, unit);
    out.local = scene;
    out.color = data.color;
    out.info = data.info;
    out.gradient_a = data.gradient_a;
    out.gradient_b = data.gradient_b;
    return out;
}

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
    let kind = u32(data.info.z);
    let first = u32(data.gradient_b.x);
    let count = u32(data.gradient_b.y);
    var t = 0.0;
    if (kind == 1u) {
        let center = data.gradient_a.xy;
        let radius = data.gradient_a.z;
        t = length(p - center) / max(radius, 1e-6);
    } else if (kind == 3u) {
        let conical = conical_position(p, data.gradient_a, data.gradient_b.zw);
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

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let p = in.local;
    var color = vec4<f32>(0.0);
    let flags = u32(data.info.x);
    if ((flags & 4u) != 0u) {
        // Decoration bar: a solid or gradient rectangle.
        if ((flags & 2u) != 0u) {
            color = gradient_color(data, p);
        } else {
            color = data.color;
        }
    } else if ((flags & 1u) != 0u) {
        // Color glyph: the atlas is straight RGBA in sRGB; the pipeline's
        // target is sRGB so the hardware converts on write.
        let texel = textureSample(color_atlas, atlas_sampler, in.uv, i32(data.info.y));
        color = vec4<f32>(texel.rgb * texel.a * data.info.w, texel.a * data.info.w);
    } else {
        let coverage = textureSample(mask_atlas, atlas_sampler, in.uv, i32(data.info.y)).r;
        var brush = data.color;
        if ((flags & 2u) != 0u) {
            brush = gradient_color(data, p);
        }
        color = brush * coverage;
    }
    color = color * clip_coverage(in.position.xy);
    return color;
}
