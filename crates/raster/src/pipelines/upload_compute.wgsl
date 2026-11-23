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
