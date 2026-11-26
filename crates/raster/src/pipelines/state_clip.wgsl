fn clip_coverage(position: vec2<f32>) -> f32 {
    let clip = active_clip;
    if clip == 0u {
        return 1.0;
    }
    let header = states[clip];
    if header.z != 0.0 {
        let bounds = states[clip + 1u];
        if any(position < bounds.xy) || any(position >= bounds.zw) {
            return 0.0;
        }
    }
    var coverage = 1.0;
    if header.x != 0.0 {
        let curves = states[clip + 3u];
        for (var i = 0u; i < u32(header.x); i += 1u) {
            let box = states[clip + 4u + i * 2u];
            let radii = states[clip + 5u + i * 2u];
            var distance: f32;
            if curves[i] > 0.0 {
                distance = sd_shaped_box(position - box.xy, box.zw, radii, curves[i]);
            } else {
                distance = sd_rounded_box(position - box.xy, box.zw, radii.x);
            }
            coverage *= clamp(0.5 - distance, 0.0, 1.0);
        }
    }
    if header.y != 0.0 {
        let mask = states[clip + 2u];
        coverage *= textureSampleLevel(clip_mask_tex, clip_mask_sampler, (position - mask.xy) * mask.zw, 0.0).r;
    }
    return coverage;
}
