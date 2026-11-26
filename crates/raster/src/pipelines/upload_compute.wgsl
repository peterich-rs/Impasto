override encoded_srgb: bool = false;

fn stored_color(color: vec4<f32>) -> vec4<f32> {
    if !encoded_srgb {
        return color;
    }
    let linear = clamp(color.rgb, vec3(0.0), vec3(1.0));
    let encoded = select(12.92 * linear,
        1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055, linear > vec3(0.0031308));
    return vec4(encoded, color.a);
}

fn tile_color(color: vec4<f32>) -> vec4<f32> {
    return select(quantizeToF16(color), color, encoded_srgb);
}

@group(0) @binding(1) var destination: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn cs_premultiply(@builtin(global_invocation_id) id: vec3<u32>) {
    if all(id.xy < textureDimensions(destination)) {
        textureStore(destination, vec2<i32>(id.xy), stored_color(premultiply(vec2<i32>(id.xy))));
    }
}

@compute @workgroup_size(8, 8)
fn cs_downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if all(id.xy < textureDimensions(destination)) {
        textureStore(destination, vec2<i32>(id.xy), stored_color(downsample(vec2<i32>(id.xy))));
    }
}

@group(0) @binding(2) var mip1: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var mip2: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var mip3: texture_storage_2d<rgba16float, write>;
var<workgroup> tile: array<vec4<f32>, 64>;

fn average_quad(index: u32, step: u32) -> vec4<f32> {
    return 0.25 * (tile[index] + tile[index + step] +
        tile[index + 8u * step] + tile[index + 9u * step]);
}

fn write_mips(pixel: vec2<u32>, local: vec2<u32>, color: vec4<f32>) {
    let index = local.y * 8u + local.x;
    textureStore(destination, vec2<i32>(pixel), stored_color(color));
    // Keep linear light in shared memory so internal levels avoid sRGB round trips.
    tile[index] = tile_color(color);
    workgroupBarrier();
    if all(local % 2u == vec2(0u)) {
        let next = average_quad(index, 1u);
        textureStore(mip1, vec2<i32>(pixel / 2u), stored_color(next));
        tile[index] = tile_color(next);
    }
    workgroupBarrier();
    if all(local % 4u == vec2(0u)) {
        let next = average_quad(index, 2u);
        textureStore(mip2, vec2<i32>(pixel / 4u), stored_color(next));
        tile[index] = tile_color(next);
    }
    workgroupBarrier();
    if index == 0u {
        textureStore(mip3, vec2<i32>(pixel / 8u), stored_color(average_quad(0u, 4u)));
    }
}

// The host admits full 8x8 tiles here, so all lanes reach every barrier.
@compute @workgroup_size(8, 8)
fn cs_premultiply_mips(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    write_mips(id.xy, local.xy, premultiply(vec2<i32>(id.xy)));
}

@compute @workgroup_size(8, 8)
fn cs_downsample_mips(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
) {
    write_mips(id.xy, local.xy, downsample(vec2<i32>(id.xy)));
}
