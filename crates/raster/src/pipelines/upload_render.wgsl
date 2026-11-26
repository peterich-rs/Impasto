@group(0) @binding(1) var linear_sampler: sampler;

@vertex
fn vs_upload(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(corners[i], 0.0, 1.0);
}

@fragment
fn fs_premultiply(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return premultiply(vec2<i32>(position.xy));
}

@fragment
fn fs_downsample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = textureDimensions(source_tex);
    let output = max(vec2(1u), size / 2u);
    let ratio = vec2<f32>(size) / vec2<f32>(output);
    if all(ratio == floor(ratio)) && all(ratio <= vec2(2.0)) {
        return textureSampleLevel(source_tex, linear_sampler, position.xy / vec2<f32>(output), 0.0);
    }
    return downsample(vec2<i32>(position.xy));
}
