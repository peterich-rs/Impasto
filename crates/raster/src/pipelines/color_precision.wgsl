// Keep positions, gradient stops and coverage calculations at F32. Only
// normalized SDR color arithmetic uses half precision.
override reduced_color_precision: bool = false;

fn mix_color(a: vec4<f32>, b: vec4<f32>, t: f32) -> vec4<f32> {
    if reduced_color_precision {
        return vec4<f32>(mix(ReducedColor(a), ReducedColor(b), ReducedScalar(t)));
    }
    return mix(a, b, t);
}

fn stroke_over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    if reduced_color_precision {
        let a = ReducedColor(src);
        return vec4<f32>(a + ReducedColor(dst) * (ReducedScalar(1.0) - a.a));
    }
    return src + dst * (1.0 - src.a);
}
