// Analytic rectangles and tessellated paths in linear premultiplied color.
override two_dimensional_shadow: bool = false;
override circular_shadow: bool = false;
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

fn rect_point(vertex: u32, data: RectVertex) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let margin = (data.shape.y * 0.5 + 1.0 / max(view.info.x, 0.001));
    return data.rect.xy - vec2(margin) + corners[vertex] * (data.rect.zw + vec2(2.0 * margin));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: RectVertex) -> VertexOut {
    return vertex_out(data, vec3<f32>(rect_point(vertex, data), 1.0));
}

struct SolidOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_radius: vec3<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs_solid(@builtin(vertex_index) vertex: u32, data: RectVertex) -> SolidOut {
    let p = rect_point(vertex, data);
    let half = data.rect.zw * 0.5;
    return SolidOut(view.transform * vec4(p, 0.0, 1.0),
        p - (data.rect.xy + half), vec3(half, data.shape.x), data.color * data.opacity);
}

@vertex
fn vs_depth_solid(@builtin(vertex_index) vertex: u32, data: RectVertex) -> SolidOut {
    let p = rect_point(vertex, data);
    let half = data.rect.zw * 0.5;
    return SolidOut(view.transform * vec4(p, data.depth, 1.0),
        p - (data.rect.xy + half), vec3(half, data.shape.x), data.color * data.opacity);
}

@fragment
fn fs_solid(in: SolidOut) -> @location(0) vec4<f32> {
    let aa = max(length(dpdx(in.local)) + length(dpdy(in.local)), 1e-6) * 0.5;
    let d = sd_rounded_box(in.local, in.half_radius.xy, in.half_radius.z);
    return in.color * clamp(0.5 - d / aa, 0.0, 1.0) * clip_coverage(in.position.xy);
}

@vertex
fn vs_mesh(@location(0) vertex: vec3<f32>, data: RectVertex) -> VertexOut {
    return vertex_out(data, vertex);
}

fn erf(x: f32) -> f32 {
    // Abramowitz-Stegun 7.1.27. Absolute error below 5e-4 is smaller
    // than a coverage step in an 8-bit target.
    let a = abs(x);
    let inverse = 1.0 / (1.0 + a * (0.278393 + a * (0.230389 + a * (0.000972 + a * 0.078108))));
    let squared = inverse * inverse;
    return sign(x) * (1.0 - squared * squared);
}

fn box_distance(data: VertexOut, p: vec2<f32>, corners: vec4<f32>) -> f32 {
    let half = data.rect.zw * 0.5;
    let centered = p - (data.rect.xy + half);
    if ((u32(data.shape.w) & 2u) != 0u) {
        return sd_shaped_box(centered, half, corners, data.curve);
    }
    return sd_rounded_box(centered, half, data.shape.x);
}

fn corner_cut(distance: f32, r: f32, curve: f32) -> f32 {
    if (r <= 0.0 || distance >= r) { return 0.0; }
    let y = clamp(1.0 - distance / r, 0.0, 1.0);
    if curve == 2.0 {
        return r * (1.0 - sqrt(max(0.0, 1.0 - y * y)));
    }
    return r * (1.0 - pow(max(0.0, 1.0 - pow(y, curve)), 1.0 / curve));
}

const shadow_nodes = array(-0.86113631, -0.33998104, 0.33998104, 0.86113631);
const shadow_weights = array(0.34785485, 0.65214515, 0.65214515, 0.34785485);
const shadow_normalizer = 0.5641895835;

fn gaussian_interval(low: f32, high: f32, center: f32, inv: f32) -> f32 {
    return 0.5 * (erf((high - center) * inv) - erf((low - center) * inv));
}

fn box_mass(half: f32, center: f32, inv: f32, halo: f32) -> f32 {
    let near = half - abs(center);
    let far = half + abs(center);
    // Most shadow pixels are far from at least one pair of edges.
    // The other tail is negligible when the far edge is at least 4 sigma away.
    if near >= halo * (7.0 / 6.0) && far >= halo * (4.0 / 3.0) { return 1.0; }
    let near_mass = erf(near * inv);
    if far >= halo * (4.0 / 3.0) { return 0.5 + 0.5 * near_mass; }
    return 0.5 * (near_mass + erf(far * inv));
}

// Subtract only the part cut from the analytic rectangle. Folding into the
// nearest quadrant shares one circular cut between the two sides of each row.
fn circular_loss(q: vec2<f32>, half: vec2<f32>, r: f32, inv: f32, halo: f32,
                 side: f32, far_x: bool) -> f32 {
    let low = max(half.y - r, side * q.y - halo);
    let high = min(half.y, side * q.y + halo);
    if high <= low { return 0.0; }
    let center = (high + low) * 0.5;
    let extent = (high - low) * 0.5;
    let right_edge = erf((half.x - q.x) * inv);
    var integral = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let y = center + extent * shadow_nodes[i];
        let t = clamp((y - (half.y - r)) / r, 0.0, 1.0);
        let cut = r * (1.0 - sqrt(max(0.0, 1.0 - t * t)));
        var loss = 0.5 * (right_edge - erf((half.x - cut - q.x) * inv));
        if far_x {
            loss += gaussian_interval(-half.x, -half.x + cut, q.x, inv);
        }
        let dy = (y - side * q.y) * inv;
        integral += shadow_weights[i] * loss * exp(-dy * dy);
    }
    return integral * extent * shadow_normalizer * inv;
}

fn shaped_loss(q: vec2<f32>, half: vec2<f32>, radii: vec4<f32>, curve: f32,
               inv: f32, halo: f32, start: f32, end: f32) -> f32 {
    let low = max(start, q.y - halo);
    let high = min(end, q.y + halo);
    if high <= low { return 0.0; }
    let center = (high + low) * 0.5;
    let extent = (high - low) * 0.5;
    var integral = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let y = center + extent * shadow_nodes[i];
        let top = y + half.y;
        let bottom = half.y - y;
        let left = max(corner_cut(top, radii.x, curve), corner_cut(bottom, radii.w, curve));
        let right = max(corner_cut(top, radii.y, curve), corner_cut(bottom, radii.z, curve));
        let loss = gaussian_interval(-half.x, -half.x + left, q.x, inv)
                 + gaussian_interval(max(half.x - right, -half.x + left), half.x, q.x, inv);
        let dy = (y - q.y) * inv;
        integral += shadow_weights[i] * loss * exp(-dy * dy);
    }
    return integral * extent * shadow_normalizer * inv;
}

struct ShadowOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half: vec2<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) shape: vec3<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn vs_shadow(@builtin(vertex_index) vertex: u32, data: RectVertex) -> ShadowOut {
    let p = rect_point(vertex, data);
    let extra = load_paint(data.paint_index);
    let sigma = max(extra.shadow.w, 0.0);
    let half = data.rect.zw * 0.5 - vec2(3.0 * sigma);
    let r = clamp(data.shape.x + extra.shadow.z, 0.0, max(0.0, min(half.x, half.y)));
    let shaped = (u32(data.shape.w) & 2u) != 0u;
    let radii = select(vec4(r), extra.corners, shaped);
    let curve = select(2.0, data.curve, shaped);
    return ShadowOut(view.transform * vec4(p, 0.0, 1.0),
        p - (data.rect.xy + data.rect.zw * 0.5), half, radii,
        vec3(sigma, curve, f32(shaped)), data.color * data.opacity);
}

fn shadow_alpha(in: ShadowOut) -> f32 {
    let half = in.half;
    if any(half <= vec2(0.0)) { return 0.0; }
    let q = in.local;
    let radii = in.radii;
    let r = radii.x;
    let sigma = in.shape.x;
    let curve = in.shape.y;
    let shaped = !circular_shadow && in.shape.z != 0.0;
    let halo = 3.0 * sigma;
    if !two_dimensional_shadow || sigma < 0.3 {
        var distance = sd_rounded_box(q, half, r);
        if shaped {
            distance = sd_shaped_box(q, half, radii, curve);
        }
        if sigma < 0.3 {
            return select(0.0, 1.0, distance <= 0.0);
        }
        return 0.5 + 0.5 * erf(-distance * 0.70710678 / sigma);
    }

    let inv = 0.70710678 / sigma;
    let box_alpha = box_mass(half.x, q.x, inv, halo) * box_mass(half.y, q.y, inv, halo);
    let radius = max(max(radii.x, radii.y), max(radii.z, radii.w));
    // Discarded one-sided Gaussian tails beyond 3 sigma have mass below 0.00135.
    if radius <= 0.0 || any(abs(q) + vec2(halo) <= half - vec2(radius)) {
        return box_alpha;
    }
    var loss = 0.0;
    if !shaped {
        let folded = abs(q);
        let far_x = folded.x + half.x - r < halo;
        loss = circular_loss(folded, half, r, inv, halo, 1.0, far_x);
        if folded.y + half.y - r < halo {
            loss += circular_loss(folded, half, r, inv, halo, -1.0, far_x);
        }
    } else {
        let top = -half.y + max(radii.x, radii.y);
        let bottom = half.y - max(radii.z, radii.w);
        if top >= bottom {
            loss = shaped_loss(q, half, radii, curve, inv, halo, -half.y, half.y);
        } else {
            loss = shaped_loss(q, half, radii, curve, inv, halo, -half.y, top)
                 + shaped_loss(q, half, radii, curve, inv, halo, bottom, half.y);
        }
    }
    return clamp(box_alpha - loss, 0.0, 1.0);
}

@fragment
fn fs_shadow(in: ShadowOut) -> @location(0) vec4<f32> {
    return in.color * shadow_alpha(in) * clip_coverage(in.position.xy);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let aa = max(length(dpdx(in.local_coverage.xy)) + length(dpdy(in.local_coverage.xy)), 1e-6) * 0.5;
    let p = in.local_coverage.xy;
    let extra = load_paint(data.paint_index);
    let kind = u32(data.shape.z);
    var color = vec4<f32>(0.0);

    if (kind == 4u) {
        // Clip mask target: write coverage of the rounded rect into r and a.
        let d = box_distance(data, p, extra.corners);
        let a = clamp(0.5 - d / aa, 0.0, 1.0);
        color = vec4<f32>(a, 0.0, 0.0, a);
    } else if (kind == 5u) {
        // Mesh: lyon wrote per-vertex coverage; the brush is the color.
        color = data.color;
    } else {
        let d = box_distance(data, p, extra.corners);
        if (kind == 0u || kind == 2u) {
            let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
            var brush = data.color;
            if ((u32(data.shape.w) & 1u) != 0u) {
                brush = paint_gradient(extra, p);
            }
            color = brush * coverage;
        }
        if (kind == 1u || kind == 2u) {
            let w = max(data.shape.y, 0.0);
            let sd = abs(d) - w * 0.5;
            let sc = clamp(0.5 - sd / aa, 0.0, 1.0);
            let src = extra.stroke * sc;
            color = stroke_over(src, color);
        }
    }

    color = color * clip_coverage(in.position.xy) * data.opacity;
    if output_dither && view.info.w > 0.0 && color.a >= 1.0 && (u32(data.shape.w) & 1u) != 0u {
        let peak = max(color.r, max(color.g, color.b));
        // Fade out dark-gradient dithering where the linear channels are bright.
        if peak < 0.65 {
            let strength = 1.0 - smoothstep(0.40, 0.65, peak);
            return vec4(mix(color.rgb, dither_linear(color.rgb, in.position.xy), strength), color.a);
        }
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
