// Uploaded source channels are straight alpha and decoded by their sRGB view.
@group(0) @binding(0) var source_tex: texture_2d<f32>;

fn premultiply(pixel: vec2<i32>) -> vec4<f32> {
    let texel = textureLoad(source_tex, pixel, 0);
    return vec4(texel.rgb * texel.a, texel.a);
}

fn downsample(pixel: vec2<i32>) -> vec4<f32> {
    let source = textureDimensions(source_tex);
    let destination = max(vec2(1u), source / 2u);
    let scale = vec2<f32>(source) / vec2<f32>(destination);
    let low = vec2<f32>(pixel) * scale;
    let high = min(low + scale, vec2<f32>(source));
    let first = vec2<i32>(floor(low));
    let end = vec2<i32>(ceil(high));
    var color = vec4(0.0);
    for (var y = first.y; y < end.y; y++) {
        for (var x = first.x; x < end.x; x++) {
            let at = vec2(x, y);
            let p = vec2<f32>(at);
            let overlap = max(vec2(0.0), min(p + 1.0, high) - max(p, low));
            color += textureLoad(source_tex, at, 0) * (overlap.x * overlap.y);
        }
    }
    return color / ((high.x - low.x) * (high.y - low.y));
}
