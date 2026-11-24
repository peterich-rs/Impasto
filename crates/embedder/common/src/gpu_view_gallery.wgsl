@group(0) @binding(0) var<uniform> info: vec4<f32>;

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let points = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(points[i], 0.0, 1.0);
}

fn spin(p: vec3<f32>) -> vec3<f32> {
    let yaw = info.z + 0.7;
    let pitch = 0.45 + 0.16 * sin(info.z);
    let q = vec3(cos(yaw) * p.x - sin(yaw) * p.z, p.y, sin(yaw) * p.x + cos(yaw) * p.z);
    return vec3(q.x, cos(pitch) * q.y - sin(pitch) * q.z, sin(pitch) * q.y + cos(pitch) * q.z);
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (2.0 * position.xy - info.xy) / min(info.x, info.y);
    let grid_distance = min(abs(fract(uv.x * 5.0 + 0.5) - 0.5), abs(fract(uv.y * 5.0 + 0.5) - 0.5));
    let grid = (1.0 - smoothstep(0.0, 0.035, grid_distance)) * 0.013;
    let background = vec3(0.017, 0.032, 0.033) + grid;
    let origin = spin(vec3(uv.x, -uv.y, 2.8));
    let direction = spin(normalize(vec3(uv * vec2(0.12, -0.12), -1.0)));
    let inverse = select(vec3(1.0), vec3(-1.0), direction < vec3(0.0)) / max(abs(direction), vec3(1e-5));
    let near = (-vec3(0.48) - origin) * inverse;
    let far = (vec3(0.48) - origin) * inverse;
    let front = min(near, far);
    let back = max(near, far);
    let t = max(max(front.x, front.y), front.z);
    if (t > min(min(back.x, back.y), back.z)) {
        return vec4(background, 1.0);
    }
    let p = origin + direction * t;
    let face = abs(p);
    let major = max(max(face.x, face.y), face.z);
    let normal = sign(p) * step(vec3(major - 1e-4), face);
    let light = max(0.0, dot(normal, spin(normalize(vec3(-0.5, 0.75, 0.8)))));
    let edge = face * (vec3(1.0) - abs(normal));
    let seam = smoothstep(0.454, 0.48, max(max(edge.x, edge.y), edge.z));
    let body = vec3(0.074, 0.43, 0.40) * (0.32 + 0.68 * light);
    return vec4(mix(body, vec3(0.58, 0.84, 0.68), seam), 1.0);
}
