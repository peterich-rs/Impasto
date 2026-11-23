// Glyphs and decoration bars.
@group(1) @binding(0) var mask_atlas: texture_2d_array<f32>;
@group(1) @binding(1) var color_atlas: texture_2d_array<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;
@group(2) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(2) @binding(1) var clip_mask_sampler: sampler;

struct GlyphVertex {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) info: vec4<f32>,
    @location(4) gradient_a: vec4<f32>,
    @location(5) gradient_b: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) info: vec4<f32>,
    @location(4) @interpolate(flat) gradient_a: vec4<f32>,
    @location(5) @interpolate(flat) gradient_b: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: GlyphVertex) -> VertexOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    let unit = corners[vertex];
    let scene = data.rect.xy + unit * data.rect.zw;
    var out: VertexOut;
    out.position = view.transform * vec4<f32>(scene, 0.0, 1.0);
    out.uv = mix(data.uv.xy, data.uv.zw, unit);
    out.local = scene;
    out.color = data.color;
    out.info = data.info;
    out.gradient_a = data.gradient_a;
    out.gradient_b = data.gradient_b;
    return out;
}

fn paint_gradient(data: VertexOut, p: vec2<f32>) -> vec4<f32> {
    let indices = vec3<u32>(vec3(data.info.z, data.gradient_b.xy));
    return gradient_color(p, indices.x, indices.y, indices.z, data.gradient_a, data.gradient_b.zw);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let p = in.local;
    var color = vec4<f32>(0.0);
    let flags = u32(data.info.x);
    if ((flags & 4u) != 0u) {
        // Decoration bar: a solid or gradient rectangle.
        if ((flags & 2u) != 0u) {
            color = paint_gradient(data, p);
        } else {
            color = data.color;
        }
    } else if ((flags & 1u) != 0u) {
        // Color glyph: the atlas is straight RGBA in sRGB; the pipeline's
        // target is sRGB so the hardware converts on write.
        let texel = textureSample(color_atlas, atlas_sampler, in.uv, i32(data.info.y));
        color = vec4<f32>(texel.rgb * texel.a * data.info.w, texel.a * data.info.w);
    } else {
        let coverage = textureSample(mask_atlas, atlas_sampler, in.uv, i32(data.info.y)).r;
        var brush = data.color;
        if ((flags & 2u) != 0u) {
            brush = paint_gradient(data, p);
        }
        color = brush * coverage;
    }
    color = color * clip_coverage(in.position.xy);
    return color;
}
