struct Factors {
    size: vec4<f32>,
    channels: array<vec4<f32>, 81>,
};

@group(0) @binding(0) var<uniform> factors: Factors;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let corners = array<vec2<f32>, 3>(vec2(0.0, 0.0), vec2(2.0, 0.0), vec2(0.0, 2.0));
    let uv = corners[index];
    return VertexOut(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let size = vec2<u32>(factors.size.xy);
    let angle = 3.14159265 * in.uv;
    var column: array<f32, 9>;
    for (var x = 0u; x < size.x; x++) {
        column[x] = cos(angle.x * f32(x));
    }
    var color = vec4(0.0);
    for (var y = 0u; y < size.y; y++) {
        let weight = cos(angle.y * f32(y));
        for (var x = 0u; x < size.x; x++) {
            color += factors.channels[y * size.x + x] * weight * column[x];
        }
    }
    if (factors.size.z == 0.0) {
        return vec4(clamp(color.rgb, vec3(0.0), vec3(1.0)), 1.0);
    }
    let l = color.x;
    let p = color.y / 3.0;
    let q = color.z * 0.5;
    let encoded = clamp(vec3(l + p + q, l + p - q, l - 2.0 * p), vec3(0.0), vec3(1.0));
    let linear = select(encoded / 12.92, pow((encoded + 0.055) / 1.055, vec3(2.4)),
        encoded > vec3(0.04045));
    let alpha = clamp(color.a, 0.0, 1.0);
    return vec4(linear * alpha, alpha);
}
