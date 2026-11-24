// RGB stripes on an opaque, axis-aligned SDR target.
struct LcdOut {
    @location(0) @blend_src(0) color: vec4<f32>,
    @location(0) @blend_src(1) coverage: vec4<f32>,
};

@fragment
fn fs_lcd(in: MaskOut) -> LcdOut {
    let layer = i32(in.flags_and_layer >> 8u);
    var masks = textureSample(mask_atlas, atlas_sampler, in.uv, layer).rgb;
    if text_gamma && (in.flags_and_layer & 8u) != 0u {
        masks = pow(masks, vec3(0.85));
    }
    let covered = max(masks.r, max(masks.g, masks.b)) * in.color.a;
    return LcdOut(vec4(in.color.rgb * masks, covered), vec4(masks * in.color.a, covered));
}
