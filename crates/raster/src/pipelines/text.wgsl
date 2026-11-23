// Glyphs and decoration bars.
override simple_paint: bool = false;
@group(1) @binding(0) var mask_atlas: texture_2d_array<f32>;
@group(1) @binding(1) var color_atlas: texture_2d_array<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;
@group(2) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(2) @binding(1) var clip_mask_sampler: sampler;

struct GlyphVertex {
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<u32>,
    @location(2) color: vec4<f32>,
    @location(3) flags_and_layer: u32,
    @location(4) paint_index: u32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) flags_and_layer: u32,
    @location(4) @interpolate(flat) paint_index: u32,
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
    out.uv = mix(vec2<f32>(data.uv.xy), vec2<f32>(data.uv.zw), unit);
    out.local = scene;
    out.color = data.color;
    out.flags_and_layer = data.flags_and_layer;
    out.paint_index = data.paint_index;
    return out;
}

fn glyph_coverage(uv: vec2<f32>, layer: i32, flags: u32) -> f32 {
    let coverage = textureSample(mask_atlas, atlas_sampler, uv, layer).r;
    return select(coverage, pow(coverage, 0.85), (flags & 8u) != 0u);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let data = in;
    let p = in.local;
    var color = vec4<f32>(0.0);
    let flags = data.flags_and_layer & 255u;
    let layer = i32(data.flags_and_layer >> 8u);
    if simple_paint {
        let uv = in.uv / vec2<f32>(textureDimensions(mask_atlas));
        return data.color * glyph_coverage(uv, layer, flags);
    }
    if ((flags & 4u) != 0u) {
        // Decoration bar: a solid or gradient rectangle.
        if ((flags & 2u) != 0u) {
            color = paint_gradient(load_paint(data.paint_index), p);
        } else {
            color = data.color;
        }
    } else if ((flags & 1u) != 0u) {
        // Color glyph: the atlas is straight RGBA in sRGB; the pipeline's
        // target is sRGB so the hardware converts on write.
        let texel = textureSample(color_atlas, atlas_sampler, in.uv / vec2<f32>(textureDimensions(color_atlas)), layer);
        color = vec4<f32>(texel.rgb * texel.a * data.color.a, texel.a * data.color.a);
    } else {
        let coverage = glyph_coverage(in.uv / vec2<f32>(textureDimensions(mask_atlas)), layer, flags);
        var brush = data.color;
        if ((flags & 2u) != 0u) {
            brush = paint_gradient(load_paint(data.paint_index), p);
        }
        color = brush * coverage;
    }
    color = color * clip_coverage(in.position.xy);
    return color;
}
