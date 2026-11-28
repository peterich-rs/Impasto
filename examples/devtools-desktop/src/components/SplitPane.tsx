import { createSignal } from "solid-js";
import { View } from "@impasto/solid-ui";
import { scene } from "@impasto/solid-renderer";
import { useTokens } from "../theme/tokens";

export function SplitHandle(props: { vertical?: boolean; resize: (delta: number) => void }) {
  const c = useTokens();
  const [active, setActive] = createSignal(false);
  return <View
    accessibilityLabel={props.vertical ? "Resize inspector" : "Resize console"}
    focusable
    ref={node => { if (node) scene.setDragClaim(node, "free"); }}
    style={{ width: props.vertical ? 5 : "100%", height: props.vertical ? "100%" : 5, flexShrink: 0,
      cursor: props.vertical ? "col-resize" : "row-resize",
      alignItems: "center", justifyContent: "center" }}
    onHover={() => setActive(true)} onLeave={() => setActive(false)}
    onDragBegin={() => setActive(true)} onDragEnd={() => setActive(false)}
    onDragBy={(dx, dy) => props.resize(props.vertical ? -dx : -dy)}
    onKey={key => {
      if (key === "Left" || key === "Up") props.resize(12);
      if (key === "Right" || key === "Down") props.resize(-12);
    }}>
    <View style={{ width: props.vertical ? 1 : "100%", height: props.vertical ? "100%" : 1,
      backgroundColor: active() ? c().accent : c().border }} />
  </View>;
}
