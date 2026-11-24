import { createSignal } from "solid-js";
import { boot } from "@impasto/solid-renderer";
import { Column, GpuView, ShaderEffect, createShaderProgram, shaderBackdropTexture } from "@impasto/solid-ui";

const program = createShaderProgram(`
  fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    return effect_sample(0u, local_pos / size);
  }
`, { animated: true });
const data = new Uint8Array([9, 1, 2, 3, 4, 9]);

boot(() => {
  const [active, setActive] = createSignal(true);
  return (
    <Column>
      <ShaderEffect
        program={active() ? program : null}
        uniforms={data.subarray(1, 5)}
        textures={["asset:fixture", shaderBackdropTexture]}
        style={{ width: 80, height: 40, padding: 4, overflow: "hidden", borderRadius: 8 }}
        onClick={() => setActive(false)}
      />
      <GpuView producer={active() ? "test-producer" : "next-producer"} style={{ width: 80, height: 40 }} />
    </Column>
  );
});
