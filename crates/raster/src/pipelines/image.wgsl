// Textured quads, including external images and resolved paths.
@group(1) @binding(0) var image_tex: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
@group(2) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(2) @binding(1) var clip_mask_sampler: sampler;

struct ImageVertex {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) info: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) info: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: ImageVertex) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let unit = corners[vertex];
    let scene = data.rect.xy + unit * data.rect.zw;
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.uv = mix(data.uv.xy, data.uv.zw, unit);
    out.color = data.color;
    out.info = data.info;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let flags = u32(data.info.x);
    var texel = textureSample(image_tex, image_sampler, in.uv);
    if ((flags & 5u) != 0u) {
        texel = textureSampleLevel(image_tex, image_sampler, in.uv, 0.0);
    }
    // Uploads and offscreen targets are already premultiplied in linear
    // space. Core Video BGRA surfaces are premultiplied in sRGB instead.
    var rgb = texel.rgb;
    if ((flags & 2u) != 0u) {
        let straight = clamp(rgb / max(texel.a, 1e-6), vec3<f32>(0.0), vec3<f32>(1.0));
        let linear = select(straight / 12.92, pow((straight + 0.055) / 1.055, vec3<f32>(2.4)), straight > vec3<f32>(0.04045));
        rgb = linear * texel.a;
    }
    var color = vec4<f32>(rgb, texel.a) * data.color;
    color = color * clip_coverage(in.position.xy);
    return color;
}
