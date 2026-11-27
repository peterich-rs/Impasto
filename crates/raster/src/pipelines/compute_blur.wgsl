// WGSL uses the same apron layout for both axes. In the vertical pass the
// invocation coordinates are transposed to keep 16 threads along the kernel.
override vertical: bool = false;
override output_srgb: bool = false;
const MAX_RADIUS: u32 = 24u;
struct BlurParams {
    size: vec2<u32>,
    radius: u32,
    sigma: f32,
};
@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var target_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: BlurParams;
alias TilePixel = vec4<f32>;
var<workgroup> tile: array<TilePixel, 512>;
var<workgroup> weights: array<f32, 25>;

@compute @workgroup_size(16, 8)
fn blur(
    @builtin(local_invocation_index) index: u32,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
) {
    let radius = min(params.radius, MAX_RADIUS);
    if index <= radius {
        let distance = f32(index) / params.sigma;
        weights[index] = exp(-0.5 * distance * distance);
    }
    let stride = 16u + 2u * radius;
    let group_size = select(vec2(16u, 8u), vec2(8u, 16u), vertical);
    let base = vec2<i32>(group.xy * group_size);
    for (var i = index; i < stride * 8u; i += 128u) {
        let along = i32(i % stride) - i32(radius);
        let across = i32(i / stride);
        let offset = select(vec2(along, across), vec2(across, along), vertical);
        let pixel = clamp(base + offset, vec2(0), vec2<i32>(params.size) - vec2(1));
        tile[i] = TilePixel(textureLoad(source_tex, pixel, 0));
    }
    workgroupBarrier();
    let pixel = base + vec2<i32>(select(local.xy, local.yx, vertical));
    if any(pixel >= vec2<i32>(params.size)) { return; }

    let center = local.y * stride + local.x + radius;
    var color = vec4<f32>(tile[center]) * weights[0];
    var total = weights[0];
    for (var tap = 1u; tap <= radius; tap++) {
        let weight = weights[tap];
        color += (vec4<f32>(tile[center - tap]) + vec4<f32>(tile[center + tap])) * weight;
        total += 2.0 * weight;
    }
    var result = color / total;
    if output_srgb {
        let rgb = max(vec3(0.0), result.rgb);
        let encoded = select(12.92 * rgb, 1.055 * pow(rgb, vec3(1.0 / 2.4)) - 0.055,
            rgb > vec3(0.0031308));
        result = vec4(encoded, result.a);
    }
    textureStore(target_tex, pixel, result);
}
