@group(0) @binding(1) var destination: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn cs_premultiply(@builtin(global_invocation_id) id: vec3<u32>) {
    if all(id.xy < textureDimensions(destination)) {
        textureStore(destination, vec2<i32>(id.xy), premultiply(vec2<i32>(id.xy)));
    }
}

@compute @workgroup_size(8, 8)
fn cs_downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if all(id.xy < textureDimensions(destination)) {
        textureStore(destination, vec2<i32>(id.xy), downsample(vec2<i32>(id.xy)));
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
    textureStore(destination, vec2<i32>(pixel), color);
    // Match the precision of a separate store/load at each level.
    tile[index] = quantizeToF16(color);
    workgroupBarrier();
    if all(local % 2u == vec2(0u)) {
        let next = average_quad(index, 1u);
        textureStore(mip1, vec2<i32>(pixel / 2u), next);
        tile[index] = quantizeToF16(next);
    }
    workgroupBarrier();
    if all(local % 4u == vec2(0u)) {
        let next = average_quad(index, 2u);
        textureStore(mip2, vec2<i32>(pixel / 4u), next);
        tile[index] = quantizeToF16(next);
    }
    workgroupBarrier();
    if index == 0u {
        textureStore(mip3, vec2<i32>(pixel / 8u), average_quad(0u, 4u));
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
