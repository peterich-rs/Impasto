// Glyphs from the mask and color atlases, plus decoration bars.
//
// A glyph instance is one quad in scene points. The fragment shader samples
// either the R8 coverage atlas (tinted by a solid or gradient brush) or the
// RGBA color atlas (emoji and COLR glyphs keep their own color).

struct View {
    transform: mat4x4<f32>,
    scale: f32,
    pad0: f32,
    pad1: f32,
};

@group(0) @binding(0) var<uniform> view: View;

struct GlyphInstance {
    // Scene points: quad x, y, w, h. The atlas rect for a mask glyph is
    // pixel-aligned so the quad covers exactly its texels.
    rect: vec4<f32>,
    // Atlas UV: u0, v0, u1, v1.
    uv: vec4<f32>,
    // Solid brush (premultiplied linear). Ignored for color glyphs.
    color: vec4<f32>,
    // bit 0: color atlas, bit 1: gradient brush, bit 2: decoration bar (no atlas).
    flags: f32,
    // Gradient kind, first stop, stop count.
    gradient_kind: f32,
    gradient_first: f32,
    gradient_count: f32,
    // Linear start.xy end.xy; radial center.xy radius start radius; sweep center.xy angles.
    gradient_a: vec4<f32>,
    // Span box for gradient coordinates: x, y, w, h in scene points.
    span: vec4<f32>,
    // Axis-aligned scissor in scene points. z == 0 disables it.
    clip_rect: vec4<f32>,
    // Coverage mask: scene rect xy, wh. z == 0 disables it.
    clip_mask: vec4<f32>,
    // Total opacity.
    opacity: f32,
    pad1: f32,
    pad2: f32,
    pad3: f32,
    pad4: vec4<f32>,
};

@group(0) @binding(1) var<uniform> stops: array<vec4<f32>, 512>;
@group(0) @binding(2) var<uniform> instances: array<GlyphInstance, 64>;
@group(0) @binding(3) var mask_atlas: texture_2d<f32>;
@group(0) @binding(4) var color_atlas: texture_2d<f32>;
@group(0) @binding(5) var atlas_sampler: sampler;
@group(0) @binding(6) var clip_mask_tex: texture_2d<f32>;
@group(0) @binding(7) var clip_mask_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) index: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let data = instances[instance];
    let unit = corners[vertex];
    let scene = data.rect.xy + unit * data.rect.zw;
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.uv = mix(data.uv.xy, data.uv.zw, unit);
    out.local = scene;
    out.index = instance;
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

fn gradient_color(data: GlyphInstance, p: vec2<f32>) -> vec4<f32> {
    let kind = u32(data.gradient_kind);
    let first = u32(data.gradient_first);
    let count = u32(data.gradient_count);
    var t = 0.0;
    if (kind == 1u) {
        let center = data.gradient_a.xy;
        let radius = data.gradient_a.z;
        t = length(p - center) / max(radius, 1e-6);
    } else if (kind == 3u) {
        let conical = conical_position(p, data.gradient_a, data.span.xy);
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

fn clip_coverage(data: GlyphInstance, position: vec2<f32>) -> f32 {
    if (data.clip_mask.z > 0.0) {
        let uv = position / vec2<f32>(view.pad0, view.pad1);
        return textureSampleLevel(clip_mask_tex, clip_mask_sampler, uv, 0.0).r;
    }
    return 1.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = instances[in.index];
    let aa = 1.0 / max(view.scale, 1e-6);
    let p = in.local;
    var color = vec4<f32>(0.0);
    let flags = u32(data.flags);
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
        let texel = textureSample(color_atlas, atlas_sampler, in.uv);
        color = vec4<f32>(texel.rgb * texel.a * data.opacity, texel.a * data.opacity);
    } else {
        let coverage = textureSample(mask_atlas, atlas_sampler, in.uv).r;
        var brush = data.color;
        if ((flags & 2u) != 0u) {
            brush = gradient_color(data, p);
        }
        color = brush * coverage;
    }
    color = color * clip_coverage(data, in.position.xy);
    return color;
}
