import { createSignal, onCleanup } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { Button, Column, Row, ScrollArea, Stack, Text, View } from "@impasto/solid-ui";
import { loc } from "../copy.ts";

export const COMPOSITOR_LAB_URL = "impasto://compositor-lab";

const INK = "#15212a";
const SECONDARY = "#46535d";
const PAGE = "#f5f7fa";
const HAIRLINE = "#dce1e7";
const SPRING = { stiffness: 14, damping: 7 };

/** Replays source motions without JS frame listeners or list virtualization. */
export function CompositorLabPage(props: { onBack?: () => void; launchUrl: string | null }): NodeHandle {
  const [status, setStatus] = createSignal(loc("准备就绪", "Ready"));
  let card: NodeHandle | null = null;
  let scroll: NodeHandle | null = null;
  let targetOnRight = false;
  let transformEnd = 0;
  let coastEnd = 0;
  const sections = [
    loc("顶部", "Top"), loc("上段", "Upper section"),
    loc("中段", "Middle section"), loc("下段", "Lower section"),
  ];

  function runTransform() {
    if (card === null) return;
    const node = card;
    const distance = Math.max(32, Math.min(72, scene.rect(node).w * 0.65));
    targetOnRight = !targetOnRight;
    const target = targetOnRight ? 1 : 0;
    const tracks = [
      ["translateX", -52 - distance, -52 + distance],
      ["translateY", -36, -20],
      ["rotate", -0.18, 0.22],
      ["scale", 1, 0.86],
      ["opacity", 242, 168],
    ] as const;
    setStatus(loc("变换进行中", "Transform running"));
    for (const [key, left, right] of tracks) {
      scene.animate(node, key, target ? left : right, target ? right : left,
        SPRING.stiffness, SPRING.damping, true);
    }
    scene.requestFrame();
  }

  function runCoast() {
    if (scroll === null) return;
    scene.stopScroll(scroll);
    scene.setScrollOffset(scroll, 0, 80);
    scene.fling(scroll, 0, -1.7);
    scene.requestFrame();
    setStatus(loc("滚动惯性进行中", "Scroll coast running"));
  }

  const auto = props.launchUrl === `${COMPOSITOR_LAB_URL}/transform`
    ? runTransform : props.launchUrl === `${COMPOSITOR_LAB_URL}/scroll` ? runCoast : null;
  const timer = auto === null ? undefined : setTimeout(auto, 500);
  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
    if (transformEnd !== 0) scene.offMotionEnd(transformEnd);
    if (coastEnd !== 0) scene.offMotionEnd(coastEnd);
  });

  return (
    <Column style={{ flex: 1, minHeight: 0, backgroundColor: PAGE }}>
      <Row style={{ height: 56, paddingLeft: 20, paddingRight: 12, alignItems: "center",
        justifyContent: "space-between", borderColor: HAIRLINE, borderWidth: 1 }}>
        <Text style={{ fontSize: 24, fontWeight: 500, color: INK }}>
          {loc("合成动画", "Compositor animation")}
        </Text>
        {props.onBack && <Button size="compact" label={loc("返回", "Back")} onClick={props.onBack} />}
      </Row>
      <Column style={{ width: "100%", maxWidth: 640, alignSelf: "center", flex: 1,
        minHeight: 0, padding: 20, gap: 12 }}>
        <Stack style={{ height: 148, backgroundColor: "#e9edf0", borderRadius: 12, overflow: "hidden",
          alignItems: "center", justifyContent: "center" }}>
          <View style={{ width: "76%", height: 1, backgroundColor: "#a7b7b8" }} />
          <View ref={(node) => {
            if (node === null) return;
            card = node;
            transformEnd = scene.onMotionEnd(node, "translateX", () => {
              setStatus(loc("变换已结束", "Transform settled"));
            });
          }}
          onClick={runTransform}
          accessibilityLabel={loc("反向播放变换", "Reverse transform")}
          style={{ position: "absolute", left: "50%", top: "50%", width: 104, height: 56,
            translateX: -52, translateY: -28, opacity: 0.95, rotate: -0.18,
            transformOriginX: "50%", transformOriginY: "50%",
            borderRadius: 8, backgroundColor: "#176356", alignItems: "center", justifyContent: "center" }}>
            <Text style={{ color: "#ffffff", fontSize: 13 }}>{loc("点按反向", "Tap to reverse")}</Text>
          </View>
        </Stack>
        <Row style={{ gap: 8, flexWrap: "wrap" }}>
          <Button size="compact" label={loc("运行变换", "Run transform")} onClick={runTransform} />
          <Button size="compact" label={loc("运行滚动", "Run coast")} onClick={runCoast} />
        </Row>
        <Text style={{ color: SECONDARY, fontSize: 13 }}>{status()}</Text>
        <ScrollArea viewportRef={(node) => {
          if (node === null) return;
          scroll = node;
          coastEnd = scene.onMotionEnd(node, "scrollY", () => {
            setStatus(loc("滚动惯性已结束", "Scroll coast settled"));
          });
        }} style={{ flex: 1, minHeight: 0, borderRadius: 12, backgroundColor: "#ffffff" }}>
          <Column style={{ gap: 0 }}>
            {sections.map((section) => (
              <Column style={{ height: 400, flexShrink: 0, padding: 12, gap: 76,
                borderWidth: 1, borderColor: HAIRLINE }}>
                <Text style={{ color: INK, fontSize: 14 }}>{section}</Text>
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
                <View style={{ width: "100%", height: 1, backgroundColor: HAIRLINE }} />
              </Column>
            ))}
          </Column>
        </ScrollArea>
      </Column>
    </Column>
  );
}
