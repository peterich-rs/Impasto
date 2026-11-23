// RGB stripes on an opaque, axis-aligned SDR target.
struct LcdOut {
    @location(0) @blend_src(0) color: vec4<f32>,
    @location(0) @blend_src(1) coverage: vec4<f32>,
};

@fragment
fn fs_lcd(in: VertexOut) -> LcdOut {
    let layer = i32(in.flags_and_layer >> 8u);
    let uv = in.uv;
    let third_pixel = dpdx(uv) / 3.0;
    let masks = vec3(
        glyph_coverage(uv - third_pixel, layer, in.flags_and_layer),
        glyph_coverage(uv, layer, in.flags_and_layer),
        glyph_coverage(uv + third_pixel, layer, in.flags_and_layer),
    );
    let covered = max(masks.r, max(masks.g, masks.b)) * in.color.a;
    return LcdOut(vec4(in.color.rgb * masks, covered), vec4(masks * in.color.a, covered));
}
