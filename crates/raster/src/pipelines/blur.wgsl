// Offscreen effects: dual-Kawase blur (downsample and upsample), a
// transparent-target composite with opacity or a separable blend, and a
// backdrop blur snapshot composite.
//
// Every pass is a full-target quad. Blur kernels are 5-tap linear-sampled
// Kawase filters; the pass count comes from the sigma.

struct View {
    transform: mat4x4<f32>,
    scale: f32,
    pad0: f32,
    pad1: f32,
};

@group(0) @binding(0) var<uniform> view: View;
// x, y: texel size of the source. z: 0 downsample, 1 upsample.
@group(0) @binding(1) var<uniform> params: vec4<f32>;
@group(0) @binding(2) var source: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vertex: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let unit = corners[vertex];
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(unit, 0.0, 1.0);
    out.uv = unit;
    return out;
}

fn kawase(uv: vec2<f32>, texel: vec2<f32>, radius: f32) -> vec4<f32> {
    let o = texel * radius;
    var sum = textureSample(source, linear_sampler, uv) * 0.25;
    // Four taps at the edge midpoints, each weighted by a bilinear split.
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(-o.x, 0.0)) * 0.125;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(o.x, 0.0)) * 0.125;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(0.0, -o.y)) * 0.125;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(0.0, o.y)) * 0.125;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(-o.x, -o.y) * 0.5) * 0.0625;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(o.x, -o.y) * 0.5) * 0.0625;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(-o.x, o.y) * 0.5) * 0.0625;
    sum = sum + textureSample(source, linear_sampler, uv + vec2<f32>(o.x, o.y) * 0.5) * 0.0625;
    return sum;
}

@fragment
fn fs_blur(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = params.xy;
    let radius = params.z;
    return kawase(in.uv, texel, radius);
}

// Composite: multiply the source by opacity. z: opacity, w: blend mode
// (0 source-over, 1 multiply, 2 screen, 3 destination-over, 4 destination-out,
// 5 lighter, 6 xor, 7 clear).
@group(1) @binding(0) var backdrop: texture_2d<f32>;
@group(1) @binding(1) var backdrop_sampler: sampler;

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4<f32> {
    let src = textureSample(source, linear_sampler, in.uv) * params.z;
    let mode = u32(params.w + 0.5);
    let dst = textureSample(backdrop, backdrop_sampler, in.uv);
    let coverage = textureSample(clip_mask, backdrop_sampler, in.uv).r;
    var result = src + dst * (1.0 - src.a);
    switch mode {
        case 1u: { result = vec4<f32>(src.rgb * dst.rgb, src.a * dst.a) + src * (1.0 - dst.a) + dst * (1.0 - src.a); }
        case 2u: { result = vec4<f32>(src.rgb + dst.rgb - src.rgb * dst.rgb, src.a + dst.a - src.a * dst.a); }
        case 3u: { result = dst + src * (1.0 - dst.a); }
        case 4u: { result = dst * (1.0 - src.a); }
        case 5u: { result = min(src + dst, vec4<f32>(1.0)); }
        case 6u: { result = src * (1.0 - dst.a) + dst * (1.0 - src.a); }
        default: {}
    }
    return mix(dst, result, coverage);
}

@fragment
fn fs_copy(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(source, linear_sampler, in.uv);
}

@fragment
fn fs_copy_srgb(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = textureSample(source, linear_sampler, in.uv);
    let rgb = select(12.92 * pixel.rgb, 1.055 * pow(pixel.rgb, vec3<f32>(1.0 / 2.4)) - 0.055, pixel.rgb > vec3<f32>(0.0031308));
    return vec4<f32>(rgb, pixel.a);
}

@group(1) @binding(2) var clip_mask: texture_2d<f32>;

@fragment
fn fs_over(in: VertexOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(clip_mask, backdrop_sampler, in.uv).r;
    return textureSample(source, linear_sampler, in.uv) * params.z * coverage;
}

@fragment
fn fs_backdrop(in: VertexOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(clip_mask, backdrop_sampler, in.uv).r;
    let dst = textureSample(backdrop, backdrop_sampler, in.uv);
    let filtered = textureSample(source, linear_sampler, in.uv);
    return mix(dst, filtered, coverage);
}
