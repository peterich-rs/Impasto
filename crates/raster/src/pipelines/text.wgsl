// Glyphs and decoration bars.
override text_gamma: bool = false;
override unclipped: bool = false;
const USE_STATE_TABLE = false;
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
    @location(4) @interpolate(flat) gradient_geometry: vec4<f32>,
    @location(5) @interpolate(flat) gradient_parameters: vec4<u32>,
    @location(6) @interpolate(flat) gradient_end: vec4<f32>,
    @location(7) @interpolate(flat) gradient_start: vec4<f32>,
    @location(8) @interpolate(flat) state: u32,
};

fn glyph_unit(vertex: u32) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0)
    );
    return corners[vertex];
}

fn glyph_uv(unit: vec2<f32>, data: GlyphVertex) -> vec2<f32> {
    var scale = vec2(view.info.w);
    if USE_STATE_TABLE || (data.flags_and_layer & 96u) != 0u {
        if (data.flags_and_layer & 1u) != 0u {
            scale = 1.0 / vec2<f32>(textureDimensions(color_atlas));
        } else {
            scale = 1.0 / vec2<f32>(textureDimensions(mask_atlas));
        }
    }
    return mix(vec2<f32>(data.uv.xy), vec2<f32>(data.uv.zw), unit) * scale;
}

struct MaskOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) flags_and_layer: u32,
    @location(3) @interpolate(flat) state: u32,
};

fn vs_mask_data(vertex: u32, data: GlyphVertex) -> MaskOut {
    let state = (data.paint_index & 0xffff0000u) | (data.flags_and_layer >> 16u);
    vertex_view(state);
    let unit = glyph_unit(vertex);
    let scene = data.rect.xy + unit * data.rect.zw;
    return MaskOut(view.transform * vec4(scene, 0.0, 1.0),
        glyph_uv(unit, data), data.color, select(data.flags_and_layer, data.flags_and_layer & 65535u, USE_STATE_TABLE), state);
}

@vertex
fn vs_mask(@builtin(vertex_index) vertex: u32, data: GlyphVertex) -> MaskOut {
    return vs_mask_data(vertex, data);
}

@fragment
fn fs_mask(in: MaskOut) -> @location(0) vec4<f32> {
    fragment_view(in.state);
    let flags = in.flags_and_layer & 255u;
    let layer = i32(in.flags_and_layer >> 8u);
    return in.color * glyph_coverage(in.uv, layer, flags) * clip_coverage(in.position.xy);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, data: GlyphVertex) -> VertexOut {
    let mask = vs_mask_data(vertex, data);
    var out: VertexOut;
    out.position = mask.position;
    out.uv = mask.uv;
    out.local = data.rect.xy + glyph_unit(vertex) * data.rect.zw;
    out.color = data.color;
    out.flags_and_layer = mask.flags_and_layer;
    out.state = mask.state;
    var index = data.paint_index;
    if USE_STATE_TABLE {
        let packed = index & 65535u;
        index = select(packed, 0xffffffffu, packed == 65535u);
    }
    let extra = load_paint(index);
    out.gradient_geometry = extra.geometry;
    out.gradient_parameters = vec4<u32>(extra.gradient);
    out.gradient_end = extra.shadow;
    out.gradient_start = extra.stroke;
    return out;
}

fn distance_coverage(uv: vec2<f32>, layer: i32, footprint: vec2<f32>) -> f32 {
        let distance = textureSampleLevel(mask_atlas, atlas_sampler, uv, layer, 0.0).rgb;
        let median = max(min(distance.r, distance.g), min(max(distance.r, distance.g), distance.b));
        let texels = vec2<f32>(textureDimensions(mask_atlas)) * footprint;
        let range = max(4.0 * (1.0 / max(texels.x, 1e-6) + 1.0 / max(texels.y, 1e-6)), 1.0);
        return clamp((median - 0.5) * range + 0.5, 0.0, 1.0);
}

fn glyph_coverage(uv: vec2<f32>, layer: i32, flags: u32) -> f32 {
    let coverage = textureSample(mask_atlas, atlas_sampler, uv, layer).r;
    if text_gamma && (flags & 8u) != 0u {
        return pow(coverage, 0.85);
    }
    return coverage;
}

fn glyph_gradient(in: VertexOut) -> vec4<f32> {
    let parameters = in.gradient_parameters;
    if parameters.x == 4u {
        return inline_gradient(in.local, in.gradient_geometry, in.gradient_start, in.gradient_end);
    }
    return gradient_color(in.local, parameters.x, parameters.y, parameters.z,
        in.gradient_geometry, in.gradient_end.xy, parameters.w, atlas_sampler);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    fragment_view(in.state);
    let data = in;
    let footprint = fwidth(in.uv);
    var color = vec4<f32>(0.0);
    let flags = data.flags_and_layer & 255u;
    let layer = i32(data.flags_and_layer >> 8u);
    if ((flags & 4u) != 0u) {
        // Decoration bar: a solid or gradient rectangle.
        if ((flags & 2u) != 0u) {
            color = glyph_gradient(in);
        } else {
            color = data.color;
        }
    } else if ((flags & 1u) != 0u) {
        // Color glyph: the atlas is straight RGBA in sRGB; the pipeline's
        // target is sRGB so the hardware converts on write.
        let texel = textureSample(color_atlas, atlas_sampler, in.uv, layer);
        color = vec4<f32>(texel.rgb * texel.a * data.color.a, texel.a * data.color.a);
    } else {
        var coverage = 0.0;
        if (flags & 64u) != 0u {
            coverage = distance_coverage(in.uv, layer, footprint);
        } else {
            coverage = glyph_coverage(in.uv, layer, flags);
        }
        var brush = data.color;
        if ((flags & 2u) != 0u) {
            brush = glyph_gradient(in);
        }
        color = brush * coverage;
    }
    if !unclipped {
        color *= clip_coverage(in.position.xy);
    }
    return color;
}
