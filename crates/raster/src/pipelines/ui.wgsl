// UI primitives: solid and gradient filled rounded rectangles, strokes,
// analytic box shadows, clip masks, and tessellated canvas meshes.
//
// Every primitive is one quad (six vertices from vertex_index) with its
// geometry in a per-instance record. The fragment shader evaluates an
// analytic signed distance field for the edge, so no offscreen pass is
// needed for a shadow or a rounded corner.
//
// All colors are linear and premultiplied. Output is premultiplied.

struct View {
    // Column-major 4x4 mapping scene points to clip space.
    transform: mat4x4<f32>,
    // Physical pixels per scene point, for antialiasing widths.
    scale: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};

@group(0) @binding(0) var<uniform> view: View;

struct RectInstance {
    // Scene points: x, y, w, h. For shadows the rect already includes the blur margin.
    rect: vec4<f32>,
    radius: f32,
    stroke_width: f32,
    // 0 fill, 1 stroke, 2 fill+stroke, 3 shadow, 4 clip mask, 5 mesh
    kind: f32,
    // bit 0: fill uses the gradient
    flags: f32,
    color: vec4<f32>,
    stroke_color: vec4<f32>,
    // Linear: start.xy, end.xy. Radial: center.xy, radius, start radius. Sweep: center.xy, start angle, end angle.
    gradient_a: vec4<f32>,
    // Gradient kind (0 linear, 1 radial, 2 sweep), first stop, stop count, opacity.
    gradient_b: vec4<f32>,
    // Shadow: offset xy, spread, sigma.
    shadow: vec4<f32>,
    // Axis-aligned scissor in scene points. z == 0 disables it.
    clip_rect: vec4<f32>,
    // Coverage mask: scene rect xy, wh. z == 0 disables it.
    clip_mask: vec4<f32>,
    pad0: vec4<f32>,
};

@group(0) @binding(1) var<uniform> stops: array<vec4<f32>, 512>;
@group(0) @binding(2) var<uniform> instances: array<RectInstance, 64>;
@group(0) @binding(3) var clip_mask_tex: texture_2d<f32>;
@group(0) @binding(4) var clip_mask_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) index: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let data = instances[instance];
    let margin = (data.stroke_width * 0.5 + 1.0 / max(view.scale, 0.001)) * data.pad0.x;
    let scene = data.rect.xy - vec2(margin) + corners[vertex] * (data.rect.zw + vec2(2.0 * margin));
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.local = scene;
    out.index = instance;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half.x, half.y));
    let q = abs(p) - half + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
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

fn gradient_color(data: RectInstance, p: vec2<f32>) -> vec4<f32> {
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
fn shadow_alpha(data: RectInstance, p: vec2<f32>) -> f32 {
    // rect is the rect grown by the shadow margin. Recover the spread box.
    let spread = data.shadow.z;
    let margin = data.shadow.w * 3.0;
    let half = data.rect.zw * 0.5 - vec2<f32>(margin);
    let center = data.rect.xy + data.rect.zw * 0.5;
    let sigma = max(data.shadow.w, 0.0);
    let r = data.radius + spread;
    let q = abs(p - center) - half + vec2<f32>(r);
    let outside = length(max(q, vec2<f32>(0.0)));
    let inside = min(max(q.x, q.y), 0.0) - r;
    let dist = outside + inside;
    if (sigma < 0.3) {
        return select(0.0, 1.0, dist <= 0.0);
    }
    return 0.5 + 0.5 * erf(-dist * 0.70710678 / sigma);
}

fn clip_coverage(data: RectInstance, position: vec2<f32>) -> f32 {
    if (data.clip_mask.z > 0.0) {
        let uv = position / vec2<f32>(view.pad0, view.pad1);
        return textureSampleLevel(clip_mask_tex, clip_mask_sampler, uv, 0.0).r;
    }
    return 1.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = instances[in.index];
    let aa = max(length(dpdx(in.local)) + length(dpdy(in.local)), 1e-6) * 0.5;
    let p = in.local;
    let kind = u32(data.kind);
    var color = vec4<f32>(0.0);

    if (kind == 3u) {
        color = data.color * shadow_alpha(data, p);
    } else if (kind == 4u) {
        // Clip mask target: write coverage of the rounded rect into r and a.
        let half = data.rect.zw * 0.5;
        let center = data.rect.xy + half;
        let d = sd_rounded_box(p - center, half, data.radius);
        let a = clamp(0.5 - d / aa, 0.0, 1.0);
        color = vec4<f32>(a, 0.0, 0.0, a);
    } else if (kind == 5u) {
        // Mesh: lyon wrote per-vertex coverage; the brush is the color.
        color = data.color;
    } else {
        let half = data.rect.zw * 0.5;
        let center = data.rect.xy + half;
        let d = sd_rounded_box(p - center, half, data.radius);
        if (kind == 0u || kind == 2u) {
            let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
            var brush = data.color;
            if ((u32(data.flags) & 1u) != 0u) {
                brush = gradient_color(data, p);
            }
            color = brush * coverage;
        }
        if (kind == 1u || kind == 2u) {
            let w = max(data.stroke_width, 0.0);
            let sd = abs(d) - w * 0.5;
            let sc = clamp(0.5 - sd / aa, 0.0, 1.0);
            let src = data.stroke_color * sc;
            color = src + color * (1.0 - src.a);
        }
    }

    color = color * clip_coverage(data, in.position.xy) * data.gradient_b.w;
    return color;
}

struct MeshOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) alpha: f32,
};

@vertex
fn vs_mesh(@location(0) vertex: vec3<f32>) -> MeshOut {
    var out: MeshOut;
    out.position = view.transform * vec4<f32>(vertex.xy, 0.0, 1.0);
    out.local = vertex.xy;
    out.alpha = vertex.z;
    return out;
}

@fragment
fn fs_mesh(in: MeshOut) -> @location(0) vec4<f32> {
    let data = instances[0];
    var color = data.color;
    if ((u32(data.flags) & 1u) != 0u) {
        color = gradient_color(data, in.local) * data.gradient_b.w;
    }
    return color * in.alpha * clip_coverage(data, in.position.xy);
}
