fn clip_coverage(position: vec2<f32>) -> f32 {
    let window_px = position + view.target_info.xy;
    var coverage = analytic_clip_coverage(window_px);
    if (view.info.z > 0.0) {
        let uv = (window_px - view.mask_rect.xy) * view.mask_rect.zw;
        coverage *= textureSampleLevel(clip_mask_tex, clip_mask_sampler, uv, 0.0).r;
    }
    return coverage * table_scissor(position);
}
