// Images: Fill, Fit, Crop, nine-slice, and canvas source rects. Every
// destination rectangle samples one source rectangle of the same texture.
// Nine-slice expands to nine instances in the same batch.

struct View {
    transform: mat4x4<f32>,
    scale: f32,
    pad0: f32,
    pad1: f32,
};

@group(0) @binding(0) var<uniform> view: View;

struct ImageInstance {
    // Destination in scene points: x, y, w, h.
    rect: vec4<f32>,
    // Source in normalized UV: u0, v0, u1, v1.
    uv: vec4<f32>,
    // Tint (premultiplied). Usually white with the draw's alpha.
    color: vec4<f32>,
    // bit 0: nearest filtering, bit 1: producer pixels premultiplied in sRGB.
    flags: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    clip_rect: vec4<f32>,
    clip_mask: vec4<f32>,
    pad3: vec4<f32>,
    pad4: vec4<f32>,
};

@group(0) @binding(1) var<uniform> instances: array<ImageInstance, 64>;
@group(0) @binding(2) var image_tex: texture_2d<f32>;
@group(0) @binding(3) var image_sampler: sampler;
@group(0) @binding(4) var clip_mask_tex: texture_2d<f32>;
@group(0) @binding(5) var clip_mask_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) index: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let data = instances[instance];
    let unit = corners[vertex];
    let scene = data.rect.xy + unit * data.rect.zw;
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.uv = mix(data.uv.xy, data.uv.zw, unit);
    out.local = scene;
    out.index = instance;
    return out;
}

fn clip_coverage(data: ImageInstance, position: vec2<f32>) -> f32 {
    if (data.clip_mask.z > 0.0) {
        let uv = position / vec2<f32>(view.pad0, view.pad1);
        return textureSampleLevel(clip_mask_tex, clip_mask_sampler, uv, 0.0).r;
    }
    return 1.0;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = instances[in.index];
    let texel = textureSample(image_tex, image_sampler, in.uv);
    // Uploads and offscreen targets are already premultiplied in linear
    // space. Core Video BGRA surfaces are premultiplied in sRGB instead.
    var rgb = texel.rgb;
    if ((u32(data.flags) & 2u) != 0u) {
        let straight = clamp(rgb / max(texel.a, 1e-6), vec3<f32>(0.0), vec3<f32>(1.0));
        let linear = select(straight / 12.92, pow((straight + 0.055) / 1.055, vec3<f32>(2.4)), straight > vec3<f32>(0.04045));
        rgb = linear * texel.a;
    }
    var color = vec4<f32>(rgb, texel.a) * data.color;
    color = color * clip_coverage(data, in.position.xy);
    return color;
}
