// Local blits, Dual-Kawase filtering and destination-reading composites.
struct Params {
    // Source origin in window pixels, followed by opacity or filter radius.
    values: vec4<f32>,
    uv_bounds: vec4<f32>,
};
@group(0) @binding(1) var<uniform> params: Params;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var linear_sampler: sampler;
@group(2) @binding(0) var backdrop: texture_2d<f32>;
@group(2) @binding(1) var backdrop_sampler: sampler;
@group(2) @binding(2) var clip_mask: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) unit: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vertex: u32) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let unit = corners[vertex];
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(unit, 0.0, 1.0);
    out.unit = unit;
    return out;
}

fn clamp_uv(uv: vec2<f32>) -> vec2<f32> {
    return clamp(uv, params.uv_bounds.xy, params.uv_bounds.zw);
}

fn read_source(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(source, linear_sampler, clamp_uv(uv), 0.0);
}

fn source_uv(position: vec2<f32>) -> vec2<f32> {
    return (position + view.target_info.xy - params.values.xy) / vec2<f32>(textureDimensions(source));
}

fn filter_uv(in: VertexOut) -> vec2<f32> {
    return in.unit * (params.uv_bounds.zw + params.uv_bounds.xy);
}

@fragment
fn fs_blur_down(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = filter_uv(in);
    let o = params.values.z * params.uv_bounds.xy;
    return 0.5 * read_source(uv) + 0.125 * (
        read_source(uv + o) + read_source(uv - o) +
        read_source(uv + vec2<f32>(o.x, -o.y)) + read_source(uv + vec2<f32>(-o.x, o.y))
    );
}

@fragment
fn fs_blur_up(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = filter_uv(in);
    let o = params.values.z * params.uv_bounds.xy;
    let cardinal =
        read_source(uv + vec2<f32>(2.0 * o.x, 0.0)) + read_source(uv - vec2<f32>(2.0 * o.x, 0.0)) +
        read_source(uv + vec2<f32>(0.0, 2.0 * o.y)) + read_source(uv - vec2<f32>(0.0, 2.0 * o.y));
    let diagonal = read_source(uv + o) + read_source(uv - o) +
        read_source(uv + vec2<f32>(o.x, -o.y)) + read_source(uv + vec2<f32>(-o.x, o.y));
    return (cardinal + 2.0 * diagonal) / 12.0;
}

fn coverage(position: vec2<f32>) -> f32 {
    let pixel = position + view.target_info.xy;
    var amount = analytic_clip_coverage(pixel);
    if (view.info.z > 0.0) {
        let uv = (pixel - view.mask_rect.xy) * view.mask_rect.zw;
        amount *= textureSampleLevel(clip_mask, backdrop_sampler, uv, 0.0).r;
    }
    return amount;
}

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = source_uv(in.position.xy);
    let src = read_source(uv) * params.values.z;
    let dst = textureSampleLevel(backdrop, backdrop_sampler, clamp_uv(uv), 0.0);
    let multiplied = src * dst + src * (1.0 - dst.a) + dst * (1.0 - src.a);
    return mix(dst, multiplied, coverage(in.position.xy));
}

@fragment
fn fs_copy(in: VertexOut) -> @location(0) vec4<f32> {
    return read_source(source_uv(in.position.xy));
}

@fragment
fn fs_copy_srgb(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = read_source(source_uv(in.position.xy));
    let rgb = select(12.92 * pixel.rgb, 1.055 * pow(pixel.rgb, vec3<f32>(1.0 / 2.4)) - 0.055, pixel.rgb > vec3<f32>(0.0031308));
    return vec4<f32>(rgb, pixel.a);
}

@fragment
fn fs_over(in: VertexOut) -> @location(0) vec4<f32> {
    return read_source(source_uv(in.position.xy)) * params.values.z * coverage(in.position.xy);
}

@fragment
fn fs_backdrop(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = source_uv(in.position.xy);
    let dst = textureSampleLevel(backdrop, backdrop_sampler, clamp_uv(uv), 0.0);
    return mix(dst, read_source(uv), coverage(in.position.xy));
}


fn dithered_output(in: VertexOut) -> vec4<f32> {
    return dither_srgb(read_source(source_uv(in.position.xy)), in.position.xy);
}

@fragment
fn fs_dither(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = dithered_output(in);
    return vec4(to_linear(pixel.rgb), pixel.a);
}

@fragment
fn fs_dither_srgb(in: VertexOut) -> @location(0) vec4<f32> {
    return dithered_output(in);
}
