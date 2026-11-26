import { createSignal } from "solid-js";
import { scene, type NodeHandle, type ViewStyle } from "@impasto/solid-renderer";
import {
  Button,
  Column,
  GpuView,
  Row,
  ScrollArea,
  ShaderEffect,
  Stack,
  Text,
  View,
  createShaderProgram,
  type ShaderEffectController,
} from "@impasto/solid-ui";
import { loc } from "../copy.ts";

export const GPU_LAB_URL = "impasto://gpu-lab";

const INK = "#15212a";
const SECONDARY = "#46535d";
const PAGE = "#f5f7fa";
const HAIRLINE = "#dce1e7";
const EFFECT_SURFACE = "#0e2429";
const VIEW_SURFACE = "#15212a";

const fullSize = { position: "absolute", top: 0, left: 0, width: "100%", height: "100%" } as const;
const viewport: ViewStyle = { width: "100%", borderRadius: 16, borderCurve: 4, overflow: "hidden" };

const diffraction = `
fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let p = (local_pos - size * 0.5) / max(size.y, 1.0);
    let a = vec2<f32>(p.x + 0.40, p.y * 1.2 + 0.15);
    let b = vec2<f32>(p.x - 0.38, p.y * 1.2 - 0.15);
    let phase = effect_time() * 1.4;
    let fringes = sin(46.0 * length(a) - 38.0 * length(b) + phase) * 0.5 + 0.5;
    let ridges = smoothstep(0.67, 0.95, fringes);
    let field = exp(-1.7 * dot(p, p));
    let low = vec3<f32>(0.009, 0.032, 0.047);
    let high = vec3<f32>(0.17, 0.66, 0.49);
    return vec4<f32>(mix(low, high, ridges * field * effect_uniform(0u)), 1.0);
}
`;

const broken = `
fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(unknown_color);
}
`;

function intensity(): Uint8Array {
  const buffer = new ArrayBuffer(4);
  new DataView(buffer).setFloat32(0, 0.9, true);
  return new Uint8Array(buffer);
}

/** Preview GPU drawing and exercise the same layout and effects as normal views. */
export function GpuLabPage(props: { onBack?: () => void; invalidOnLaunch?: boolean }): NodeHandle {
  const [moving, setMoving] = createSignal(!scene.reduceMotion());
  const [invalid, setInvalid] = createSignal(props.invalidOnLaunch === true);
  const [backdrop, setBackdrop] = createSignal(true);
  const activeProgram = createShaderProgram(diffraction, { animated: true });
  const stillProgram = createShaderProgram(diffraction);
  const brokenProgram = createShaderProgram(broken);
  const uniforms = intensity();
  let controller: ShaderEffectController | null = null;
  let nextIntensity = 0.25;

  return (
    <Column style={{ flex: 1, minHeight: 0, backgroundColor: PAGE }}>
      <Row style={{ minHeight: 56, paddingLeft: 20, paddingRight: 12, alignItems: "center",
        justifyContent: "space-between", borderColor: HAIRLINE, borderWidth: 1 }}>
        <Text style={{ fontSize: 24, fontWeight: 500, color: INK }}>
          {loc("GPU 实验室", "GPU Lab")}
        </Text>
        {props.onBack && <Button size="compact" label={loc("返回", "Back")} onClick={props.onBack} />}
      </Row>
      <ScrollArea style={{ flex: 1, minHeight: 0 }}>
        <Column style={{ width: "100%", maxWidth: 640, alignSelf: "center", padding: 20, gap: 20 }}>
          <Column style={{ gap: 8 }}>
            <Text style={{ fontSize: 16, fontWeight: 500, color: INK }}>Runtime Effect</Text>
            <Text style={{ fontSize: 13, color: SECONDARY }}>
              {loc("WGSL 衍射条纹 · 合成时钟", "WGSL diffraction field · compositor clock")}
            </Text>
            <Stack style={{ ...viewport, height: 176, backgroundColor: EFFECT_SURFACE }}>
              <View style={{ ...fullSize, alignItems: "center", justifyContent: "center" }}>
                <Text style={{ color: "#d8e4e2", fontSize: 14 }}>
                  {loc("应用背景", "Application background")}
                </Text>
              </View>
              <ShaderEffect
                program={invalid() ? brokenProgram : moving() ? activeProgram : stillProgram}
                uniforms={uniforms}
                style={fullSize}
                accessibilityLabel={loc("WGSL 衍射条纹", "WGSL diffraction field")}
                controller={(value) => { controller = value; }}
              />
            </Stack>
            <Row style={{ gap: 8, flexWrap: "wrap" }}>
              <Button
                size="compact"
                label={moving() ? loc("暂停条纹", "Pause field") : loc("播放条纹", "Animate field")}
                disabled={invalid()}
                onClick={() => setMoving((value) => !value)}
              />
              <Button
                size="compact"
                label={invalid() ? loc("恢复 WGSL", "Restore WGSL") : loc("尝试无效 WGSL", "Try invalid WGSL")}
                onClick={() => setInvalid((value) => !value)}
              />
              <Button
                size="compact"
                label={loc("调整条纹强度", "Sweep intensity")}
                disabled={invalid()}
                onClick={() => {
                  controller?.animateUniform(0, nextIntensity, 1200);
                  nextIntensity = nextIntensity === 0.25 ? 0.9 : 0.25;
                }}
              />
            </Row>
          </Column>
          <Column style={{ gap: 8 }}>
            <Row style={{ justifyContent: "space-between", alignItems: "center", gap: 8 }}>
              <Text style={{ fontSize: 16, fontWeight: 500, color: INK }}>GPU View</Text>
              <Button
                size="compact"
                label={backdrop() ? loc("关闭背景模糊", "Hide blur") : loc("显示背景模糊", "Show blur")}
                onClick={() => setBackdrop((value) => !value)}
              />
            </Row>
            <Stack style={{ ...viewport, height: 160, backgroundColor: VIEW_SURFACE }}>
              <View style={{ ...fullSize, alignItems: "center", justifyContent: "center" }}>
                <Text style={{ color: "#d8e4e2", fontSize: 14 }}>
                  {loc("等待原生帧", "Waiting for native frames")}
                </Text>
              </View>
              <GpuView
                producer="gallery-demo"
                style={{ ...fullSize, opacity: 0.96 }}
                accessibilityLabel={loc("同设备原生渲染", "Same-device native rendering")}
              />
              {backdrop() && <View style={{
                position: "absolute", bottom: 8, right: 8, height: 40, paddingLeft: 8, paddingRight: 8,
                justifyContent: "center", borderRadius: 8, backdrop: 8, backgroundColor: "#15212a66",
              }}>
                <Text style={{ color: "#ffffff", fontSize: 12 }}>{loc("背景模糊", "Backdrop blur")}</Text>
              </View>}
            </Stack>
          </Column>
        </Column>
      </ScrollArea>
    </Column>
  );
}
