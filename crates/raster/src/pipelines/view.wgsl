struct View {
    transform: mat4x4<f32>,
    info: vec4<f32>,
    target_info: vec4<f32>,
    mask_rect: vec4<f32>,
    clip_boxes: array<vec4<f32>, 4>,
    clip_radii: array<vec4<f32>, 4>,
    clip_curves: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;

fn to_srgb(value: vec3<f32>) -> vec3<f32> {
    return select(12.92 * value, 1.055 * pow(value, vec3(1.0 / 2.4)) - 0.055, value > vec3(0.0031308));
}

fn to_linear(value: vec3<f32>) -> vec3<f32> {
    return select(value / 12.92, pow((value + 0.055) / 1.055, vec3(2.4)), value > vec3(0.04045));
}

fn dither_srgb(pixel: vec4<f32>, position: vec2<f32>) -> vec4<f32> {
    let noise = fract(52.9829189 * fract(dot(floor(position), vec2(0.06711056, 0.00583715)))) - 0.5;
    let rgb = clamp(to_srgb(pixel.rgb) + vec3(noise / 255.0), vec3(0.0), to_srgb(vec3(pixel.a)));
    return vec4(rgb, pixel.a);
}

fn dither_linear(rgb: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    // Approximate the local sRGB EOTF slope. At half a code of noise the
    // two-term fit differs by less than a tenth of an SDR code.
    let noise = fract(52.9829189 * fract(dot(floor(position),
        vec2(0.06711056, 0.00583715)))) - 0.5;
    let root = sqrt(max(rgb, vec3(0.0)));
    let slope = select(vec3(1.0 / 12.92), (2.4 / 1.055) * root * (0.7 + 0.3 * root),
        rgb > vec3(0.0031308));
    return max(rgb + slope * (noise / 255.0), vec3(0.0));
}

fn sd_rounded_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half.x, half.y));
    let q = abs(p) - half + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

// The four corner domains are anchored to their own outer edges. Adjacent
// radii have already been fitted together in scene points on the CPU.
fn sd_shaped_box(p: vec2<f32>, half: vec2<f32>, radii: vec4<f32>, curve: f32) -> f32 {
    var distance = sd_rounded_box(p, half, 0.0);
    let directions = array<vec2<f32>, 4>(vec2(-1.0, -1.0), vec2(1.0, -1.0),
        vec2(1.0, 1.0), vec2(-1.0, 1.0));
    for (var i = 0u; i < 4u; i++) {
        let radius = radii[i];
        let from_edge = half - p * directions[i];
        if (radius > 0.0 && all(from_edge < vec2(radius))) {
            let q = vec2(1.0) - from_edge / radius;
            let norm = pow(pow(q.x, curve) + pow(q.y, curve), 1.0 / curve);
            distance = max(distance, (norm - 1.0) * radius);
        }
    }
    return distance;
}

fn analytic_clip_coverage(window_px: vec2<f32>) -> f32 {
    var coverage = 1.0;
    for (var i = 0u; i < u32(view.info.y); i += 1u) {
        let b = view.clip_boxes[i];
        var d = 0.0;
        if (view.clip_curves[i] > 0.0) {
            d = sd_shaped_box(window_px - b.xy, b.zw, view.clip_radii[i], view.clip_curves[i]);
        } else {
            d = sd_rounded_box(window_px - b.xy, b.zw, view.clip_radii[i].x);
        }
        coverage *= clamp(0.5 - d, 0.0, 1.0);
    }
    return coverage;
}
