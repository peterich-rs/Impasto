@group(0) @binding(1) var<uniform> stops: array<vec4<f32>, 512>;

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

fn gradient_color(
    p: vec2<f32>, kind: u32, first: u32, count: u32, geometry: vec4<f32>, radii: vec2<f32>,
) -> vec4<f32> {
    var t = 0.0;
    if (kind == 1u) {
        let center = geometry.xy;
        let radius = geometry.z;
        t = length(p - center) / max(radius, 1e-6);
    } else if (kind == 3u) {
        let conical = conical_position(p, geometry, radii);
        return sample_gradient(conical.x, first, count) * conical.y;
    } else if (kind == 2u) {
        let center = geometry.xy;
        let start = geometry.z;
        let end = geometry.w;
        let angle = atan2(p.y - center.y, p.x - center.x) - start;
        let wrapped = angle - 6.28318530718 * floor(angle / 6.28318530718);
        t = wrapped / max(end - start, 1e-6);
    } else {
        let start = geometry.xy;
        let end = geometry.zw;
        let d = end - start;
        t = dot(p - start, d) / max(dot(d, d), 1e-6);
    }
    return sample_gradient(t, first, count);
}
