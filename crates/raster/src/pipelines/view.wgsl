struct View {
    transform: mat4x4<f32>,
    info: vec4<f32>,
    target_info: vec4<f32>,
    mask_rect: vec4<f32>,
    clip_boxes: array<vec4<f32>, 4>,
    clip_radii: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;

fn sd_rounded_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half.x, half.y));
    let q = abs(p) - half + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

fn analytic_clip_coverage(window_px: vec2<f32>) -> f32 {
    var coverage = 1.0;
    for (var i = 0u; i < u32(view.info.y); i += 1u) {
        let b = view.clip_boxes[i];
        let d = sd_rounded_box(window_px - b.xy, b.zw, view.clip_radii[i]);
        coverage *= clamp(0.5 - d, 0.0, 1.0);
    }
    return coverage;
}
