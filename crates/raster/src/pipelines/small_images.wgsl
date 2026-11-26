@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    fragment_view(in.state);
    let start = u32(in.info.z);
    let extent = u32(in.info.w);
    let origin = vec2<f32>(f32(start & 2047u), f32(start >> 11u));
    let size = vec2<f32>(f32(extent & 2047u), f32(extent >> 11u));
    let flags = in.info.x & 255u;
    let footprint = max(length(dpdx(in.uv * size)), length(dpdy(in.uv * size)));
    let level = select(clamp(round(log2(max(1.0, footprint))), 0.0, f32((flags >> 3u) & 3u)),
        0.0, (flags & 5u) != 0u);
    let divisor = exp2(level);
    let mip_size = max(vec2<f32>(1.0), floor(size / divisor));
    // Clamp filtering to the source's texel centers. Neighboring allocations
    // can have unrelated colors, so sampling at the allocation edge must stay inside.
    let texels = origin / divisor + clamp(in.uv * mip_size, vec2<f32>(0.5), mip_size - 0.5);
    let uv = texels * divisor / vec2<f32>(textureDimensions(image_tex));
    let color = textureSampleLevel(image_tex, image_sampler, uv, i32(in.info.x >> 8u), level);
    return color * in.color * clip_coverage(in.position.xy);
}
