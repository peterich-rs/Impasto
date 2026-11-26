struct Nv12Conversion {
    matrix: mat4x4<f32>,
    chroma_offset: vec4<f32>,
};
@group(1) @binding(2) var chroma_plane: texture_2d<f32>;
@group(1) @binding(3) var<uniform> conversion: Nv12Conversion;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    fragment_view(in.state);
    let y = textureSampleLevel(image_tex, image_sampler, in.uv, 0.0).r;
    let uv = textureSampleLevel(chroma_plane, image_sampler, in.uv + conversion.chroma_offset.xy, 0.0).rg;
    let rgb = (conversion.matrix * vec4(y, uv, 1.0)).rgb;
    return vec4(to_linear(clamp(rgb, vec3(0.0), vec3(1.0))), 1.0)
        * in.color * clip_coverage(in.position.xy);
}
