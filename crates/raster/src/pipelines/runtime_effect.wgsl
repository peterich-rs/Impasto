struct EffectParams {
    uniforms: array<vec4<f32>, 8>,
    mappings: array<vec4<f32>, 4>,
    sample_modes: vec4<u32>,
};

@group(0) @binding(1) var<uniform> effect_params: EffectParams;
@group(1) @binding(0) var effect_tex_0: texture_2d<f32>;
@group(1) @binding(1) var effect_tex_1: texture_2d<f32>;
@group(1) @binding(2) var effect_tex_2: texture_2d<f32>;
@group(1) @binding(3) var effect_tex_3: texture_2d<f32>;
@group(1) @binding(4) var effect_sampler: sampler;
@group(2) @binding(0) var clip_mask_tex: texture_2d<f32>;
@group(2) @binding(1) var clip_mask_sampler: sampler;

fn effect_uniform(index: u32) -> f32 {
    if (index >= 32u) { return 0.0; }
    return effect_params.uniforms[index / 4u][index % 4u];
}

fn effect_time() -> f32 {
    return view.info.w;
}

fn effect_sample(index: u32, uv: vec2<f32>) -> vec4<f32> {
    if (index >= 4u || effect_params.sample_modes[index] == 0u) {
        return vec4<f32>(0.0);
    }
    let mapping = effect_params.mappings[index];
    let coord = mix(mapping.xy, mapping.zw, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    var sampled: vec4<f32>;
    switch index {
        case 0u: { sampled = textureSampleLevel(effect_tex_0, effect_sampler, coord, 0.0); }
        case 1u: { sampled = textureSampleLevel(effect_tex_1, effect_sampler, coord, 0.0); }
        case 2u: { sampled = textureSampleLevel(effect_tex_2, effect_sampler, coord, 0.0); }
        default: { sampled = textureSampleLevel(effect_tex_3, effect_sampler, coord, 0.0); }
    }
    var straight = sampled.rgb / max(sampled.a, 1e-6);
    if (effect_params.sample_modes[index] == 2u) {
        straight = to_linear(clamp(straight, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return vec4<f32>(straight, sampled.a);
}

struct EffectVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) size: vec2<f32>,
};

@vertex
fn effect_vertex(@builtin(vertex_index) vertex: u32, @location(0) bounds: vec4<f32>) -> EffectVertex {
    let corners = array(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
        vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let local = corners[vertex] * bounds.zw;
    return EffectVertex(view.transform * vec4(bounds.xy + local, 0.0, 1.0), local, bounds.zw);
}

@fragment
fn effect_fragment(vertex: EffectVertex) -> @location(0) vec4<f32> {
    let straight = effect_main(vertex.local, vertex.size);
    let coverage = clip_coverage(vertex.position.xy);
    let alpha = clamp(straight.a, 0.0, 1.0) * coverage;
    return vec4(clamp(straight.rgb, vec3(0.0), vec3(16.0)) * alpha, alpha);
}
