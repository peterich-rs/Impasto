// Local blits, Dual-Kawase filtering and destination-reading composites.
struct Params {
    // Source origin in window pixels, followed by opacity or filter radius.
    values: vec4<f32>,
    uv_bounds: vec4<f32>,
    backdrop_origin: vec4<f32>,
    backdrop_bounds: vec4<f32>,
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
    let center = read_source(uv);
    if reduced_color_precision {
        return vec4<f32>(ReducedScalar(0.5) * ReducedColor(center)
            + ReducedScalar(0.125) * diagonal_samples_half(uv, o));
    }
    return 0.5 * center + 0.125 * diagonal_samples(uv, o);
}

@fragment
fn fs_blur_up(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = filter_uv(in);
    let o = params.values.z * params.uv_bounds.xy;
    let x = vec2(2.0 * o.x, 0.0);
    let y = vec2(0.0, 2.0 * o.y);
    let a = read_source(uv + x);
    let b = read_source(uv - x);
    let c = read_source(uv + y);
    let d = read_source(uv - y);
    if reduced_color_precision {
        let cardinal = ReducedColor(a) + ReducedColor(b) + ReducedColor(c) + ReducedColor(d);
        return vec4<f32>((cardinal + ReducedScalar(2.0) * diagonal_samples_half(uv, o))
            / ReducedScalar(12.0));
    }
    return (a + b + c + d + 2.0 * diagonal_samples(uv, o)) / 12.0;
}

fn diagonal_samples(uv: vec2<f32>, o: vec2<f32>) -> vec4<f32> {
    let reflected = vec2(o.x, -o.y);
    return read_source(uv + o) + read_source(uv - o)
        + read_source(uv + reflected) + read_source(uv - reflected);
}

fn diagonal_samples_half(uv: vec2<f32>, o: vec2<f32>) -> ReducedColor {
    let reflected = vec2(o.x, -o.y);
    return ReducedColor(read_source(uv + o)) + ReducedColor(read_source(uv - o))
        + ReducedColor(read_source(uv + reflected)) + ReducedColor(read_source(uv - reflected));
}

// Pair adjacent Gaussian taps with bilinear sampling. Sending a fractional
// sigma continuously changes the integrated weights at the kernel edge.
@fragment
fn fs_gaussian(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = source_uv(in.position.xy);
    let sigma = max(params.values.z, 0.01);
    let axis = select(vec2(1.0, 0.0), vec2(0.0, 1.0), params.values.w != 0.0);
    let texel = axis / vec2<f32>(textureDimensions(source));
    let step = max(1.0, sigma / 8.0);
    let spacing = step / sigma;
    let radius = min(24.0, 3.0 / spacing);
    var sum = read_source(uv);
    var total = 1.0;
    for (var i = 1.0; i < radius + 0.5; i += 2.0) {
        let x = i * spacing;
        let y = (i + 1.0) * spacing;
        let first = exp(-0.5 * x * x) * clamp(radius + 0.5 - i, 0.0, 1.0);
        let second = exp(-0.5 * y * y) * clamp(radius - 0.5 - i, 0.0, 1.0);
        let weight = first + second;
        let offset = texel * step * (i + second / max(weight, 1e-6));
        sum += weight * (read_source(uv + offset) + read_source(uv - offset));
        total += 2.0 * weight;
    }
    return sum / total;
}

fn read_backdrop(position: vec2<f32>) -> vec4<f32> {
    let uv = (position + view.target_info.xy - params.backdrop_origin.xy)
        / vec2<f32>(textureDimensions(backdrop));
    return textureSampleLevel(backdrop, backdrop_sampler,
        clamp(uv, params.backdrop_bounds.xy, params.backdrop_bounds.zw), 0.0);
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
    let dst = read_backdrop(in.position.xy);
    let multiplied = src * dst + src * (1.0 - dst.a) + dst * (1.0 - src.a);
    return mix(dst, multiplied, coverage(in.position.xy));
}

fn surface_alpha(pixel: vec4<f32>) -> vec4<f32> {
    if params.values.w != 0.0 && pixel.a > 0.0 {
        return vec4(pixel.rgb / pixel.a, pixel.a);
    }
    return pixel;
}

@fragment
fn fs_copy(in: VertexOut) -> @location(0) vec4<f32> {
    return surface_alpha(read_source(source_uv(in.position.xy)));
}

@fragment
fn fs_copy_srgb(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = surface_alpha(read_source(source_uv(in.position.xy)));
    let rgb = select(12.92 * pixel.rgb, 1.055 * pow(pixel.rgb, vec3<f32>(1.0 / 2.4)) - 0.055, pixel.rgb > vec3<f32>(0.0031308));
    return vec4<f32>(rgb, pixel.a);
}

fn display_p3(position: vec2<f32>) -> vec4<f32> {
    let pixel = read_source(source_uv(position));
    let rgb = mat3x3<f32>(
        vec3(0.8224620, 0.0331942, 0.0170826),
        vec3(0.1775380, 0.9668058, 0.0723974),
        vec3(0.0, 0.0, 0.9105200),
    ) * pixel.rgb;
    return vec4(clamp(rgb, vec3(0.0), vec3(pixel.a)), pixel.a);
}

@fragment
fn fs_p3_linear(in: VertexOut) -> @location(0) vec4<f32> {
    return surface_alpha(display_p3(in.position.xy));
}

@fragment
fn fs_p3_encoded(in: VertexOut) -> @location(0) vec4<f32> {
    let pixel = surface_alpha(display_p3(in.position.xy));
    return vec4(to_srgb(pixel.rgb), pixel.a);
}

@fragment
fn fs_over(in: VertexOut) -> @location(0) vec4<f32> {
    return read_source(source_uv(in.position.xy)) * params.values.z * coverage(in.position.xy);
}

@fragment
fn fs_backdrop(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = source_uv(in.position.xy);
    let dst = read_backdrop(in.position.xy);
    return mix(dst, read_source(uv), coverage(in.position.xy));
}


fn dithered_output(in: VertexOut) -> vec4<f32> {
    let pixel = read_source(source_uv(in.position.xy));
    if params.values.w != 0.0 {
        let straight = surface_alpha(pixel);
        return vec4(dither_srgb(vec4(straight.rgb, 1.0), in.position.xy).rgb, pixel.a);
    }
    return dither_srgb(pixel, in.position.xy);
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
