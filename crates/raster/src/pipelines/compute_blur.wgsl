// A tile with an apron of at most 24 pixels for separable Gaussian filtering.
override vertical: bool = false;
struct BlurParams {
    size: vec2<u32>,
    radius: u32,
    sigma: f32,
};
@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var target_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: BlurParams;
alias TilePixel = vec4<f32>;
var<workgroup> tile: array<TilePixel, 448>;

@compute @workgroup_size(8, 8)
fn blur(
    @builtin(local_invocation_index) index: u32,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(global_invocation_id) global: vec3<u32>,
) {
    let stride = 8u + 2u * params.radius;
    for (var i = index; i < stride * 8u; i += 64u) {
        let along = i32(i % stride) - i32(params.radius);
        let across = i32(i / stride);
        let offset = select(vec2(along, across), vec2(across, along), vertical);
        let pixel = clamp(vec2<i32>(group.xy * 8u) + offset,
            vec2(0), vec2<i32>(params.size) - vec2(1));
        tile[i] = TilePixel(textureLoad(source_tex, pixel, 0));
    }
    workgroupBarrier();
    if any(global.xy >= params.size) { return; }

    let position = select(local.x, local.y, vertical);
    let row = select(local.y, local.x, vertical) * stride;
    var color = vec4(0.0);
    var total = 0.0;
    for (var tap = 0u; tap <= params.radius * 2u; tap++) {
        let distance = f32(i32(tap) - i32(params.radius)) / params.sigma;
        let weight = exp(-0.5 * distance * distance);
        color += vec4<f32>(tile[row + position + tap]) * weight;
        total += weight;
    }
    textureStore(target_tex, vec2<i32>(global.xy), color / total);
}
