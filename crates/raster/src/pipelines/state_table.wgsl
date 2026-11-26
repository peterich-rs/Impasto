// Both variable-length tables occupy one arena page. The instance packs
// two 16-bit offsets, in units of 16 bytes.
@group(0) @binding(4) var<storage, read> states: array<vec4<f32>>;
var<private> view: View;
var<private> active_clip: u32;

fn vertex_view(id: u32) {
    let transform = id & 65535u;
    let clip = id >> 16u;
    let linear = states[transform];
    let geometry = states[transform + 1u];
    view.transform = mat4x4(vec4(linear.xy, 0.0, 0.0), vec4(linear.zw, 0.0, 0.0),
        vec4(0.0, 0.0, 1.0, 0.0), vec4(geometry.xy, 0.0, 1.0));
    view.info.x = geometry.z;
    if clip != 0u {
        view.info.y = states[clip].x;
        view.info.z = states[clip].y;
    }
    view.target_info.z = geometry.w;
}

fn fragment_view(id: u32) {
    active_clip = id >> 16u;
}
