import { For, Show, createSignal, onCleanup } from "solid-js";
import { onLayout, scene, type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import { Button } from "./Button.tsx";
import { Row } from "./Row.tsx";
import type { TabsProps } from "./props.ts";
import { bindScrollStyle, interpolate } from "./scroll-style.ts";
import { useTheme } from "./use-theme.ts";

/** In-flow tabs. A horizontal pager can drive the indicator directly. */
export function Tabs(props: TabsProps): NodeHandle {
  const theme = useTheme();
  const [indicatorWidth, setIndicatorWidth] = createSignal(0);
  const buttons = new Map<string, NodeHandle>();
  let row = 0;
  let marker = 0;
  let last = "";
  let stopBinding = () => {};
  function updateIndicator(): void {
    const pager = props.pager;
    const viewport = pager?.node() ?? 0;
    if (!pager || !viewport || !row || !marker || props.tabs.length === 0) return;
    const page = scene.rect(viewport).w;
    const rowX = scene.rect(row).x;
    const tabs = props.tabs.map((tab) => buttons.get(tab.id));
    if (page <= 0 || tabs.some((node) => node === undefined)) return;
    const boxes = tabs.map((node) => scene.rect(node!));
    const base = boxes[0]!.w;
    if (base <= 0 || boxes.some((box) => box.w <= 0)) return;
    const x = boxes.map((box) => box.x - rowX);
    const widths = boxes.map((box) => box.w / base);
    const key = JSON.stringify([viewport, marker, page, x, widths, base]);
    if (key === last) return;
    last = key;
    setIndicatorWidth(base);
    stopBinding();
    if (boxes.length === 1) {
      scene.patchStyle(marker, { translateX: x[0] });
      return;
    }
    const input = boxes.map((_, index) => index * page);
    stopBinding = bindScrollStyle(marker, pager, {
      source: "scrollX",
      translateX: interpolate(input, x),
      scaleX: interpolate(input, widths),
    });
  }
  const stopLayout = onLayout(updateIndicator);
  onCleanup(() => {
    stopLayout();
    stopBinding();
  });
  return (
    <view
      {...eventsOf(props)}
      ref={(node) => bindBox(node, props, () => ({ gap: theme().space.s }))}
    >
      <Row ref={(node) => { row = node ?? 0; }} style={{ gap: theme().space.s }}>
        <For each={props.tabs}>
          {(tab, index) => (
            <Button
              ref={(node) => {
                if (node === null) buttons.delete(tab.id);
                else buttons.set(tab.id, node);
              }}
              label={tab.label}
              variant={!props.pager && tab.id === props.selected ? "primary" : "quiet"}
              onClick={() => {
                const pager = props.pager;
                const node = pager?.node() ?? 0;
                if (node !== 0) pager?.scrollTo(index() * scene.rect(node).w, 0, { stiffness: 220, damping: 30 });
                props.onSelect(tab.id);
              }}
              class={props.tabProps?.class}
              classList={props.tabProps?.classList}
              style={props.tabProps?.style}
            />
          )}
        </For>
        <Show when={props.pager}>
          <view
            ref={(node) => { marker = node ?? 0; }}
            style={{
              position: "absolute", bottom: 0, left: 0, height: 2,
              width: indicatorWidth(), backgroundColor: theme().color.primaryContainer,
              transformOriginX: 0, transformOriginY: 0,
            }}
          />
        </Show>
      </Row>
      <stack class={props.panelProps?.class} classList={props.panelProps?.classList} style={props.panelProps?.style}>
        {props.children}
      </stack>
    </view>
  );
}
