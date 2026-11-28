import { For, Show, createMemo, createSignal, createTrackedEffect, onCleanup, type Accessor } from "solid-js";
import { Pressable, Row, ScrollArea, View, createScrollController, type UIChildren } from "@impasto/solid-ui";
import { onLayout, scene, type NodeHandle } from "@impasto/solid-renderer";
import type { Row as Entry } from "@app/devtools-core";
import { useTokens } from "../theme/tokens";
import { Empty, Label, Tool } from "./ui";

const ROW = 26;
const OVERSCAN = 7;

export function SliceTable<T extends { id: string }>(props: {
  query: (offset: number, limit: number) => { total: number; rows: readonly T[] };
  render: (row: Accessor<T>) => UIChildren;
  empty?: string;
  locate?: number;
  follow?: boolean;
  reset?: string;
}) {
  const scroll = createScrollController();
  const [offset, setOffset] = createSignal(0);
  const [height, setHeight] = createSignal(260);
  const [following, setFollowing] = createSignal(true);
  const range = createMemo(() => Math.max(0, Math.floor(offset() / ROW) - OVERSCAN));
  const slice = createMemo(() => props.query(range(), Math.ceil(height() / ROW) + OVERSCAN * 2));
  const total = createMemo(() => slice().total);
  const bottom = () => Math.max(0, total() * ROW - height());
  let viewport: NodeHandle | null = null;
  let programmatic = -1;
  const jump = (y: number) => {
    programmatic = y;
    scroll.scrollTo(0, y);
  };
  onCleanup(onLayout(() => {
    if (viewport) setHeight(scene.rect(viewport).h);
  }));
  createTrackedEffect(() => {
    const index = props.locate;
    if (index != null) jump(Math.max(0, index * ROW - height() / 2 + ROW));
  });
  createTrackedEffect(() => {
    if (props.follow && following()) {
      const y = bottom();
      queueMicrotask(() => jump(y));
    }
  });
  let reset: string | undefined;
  createTrackedEffect(() => {
    if (props.reset !== reset) {
      reset = props.reset;
      jump(0);
      setOffset(0);
    }
  });

  return <View style={{ flex: 1, minHeight: 0, minWidth: 0, position: "relative" }}>
    <ScrollArea
      style={{ flex: 1, minHeight: 0, minWidth: 0 }}
      viewportRef={node => { viewport = node; }}
      controller={scroll}
      contentMin={total() * ROW}
      onOffset={point => {
        setOffset(point.y);
        if (props.follow && following() && point.y < bottom() - ROW && Math.abs(point.y - programmatic) > 1) setFollowing(false);
      }}
      contentProps={{ style: { position: "relative", width: "100%" } }}>
      <For each={slice().rows} keyed={row => row.id}>{(item, index) => <View style={{
        position: "absolute", top: (range() + index()) * ROW, left: 0, right: 0, height: ROW,
      }}>{props.render(item)}</View>}</For>
    </ScrollArea>
    <Show when={total() === 0}>
      <View style={{ position: "absolute", top: 0, left: 0, right: 0 }}><Empty title={props.empty ?? "No matching records"} /></View>
    </Show>
    <Show when={props.follow && !following() && total() > 0}>
      <View style={{ position: "absolute", bottom: 6, right: 10 }}>
        <Tool label="Jump to latest" active onClick={() => { setFollowing(true); jump(bottom()); }} />
      </View>
    </Show>
  </View>;
}

export function TableRow(props: {
  row: Entry;
  selected?: boolean;
  onClick?: () => void;
  onExpand?: () => void;
  expanded?: boolean;
  onHover?: () => void;
  onLeave?: () => void;
  children?: UIChildren;
}) {
  const c = useTokens();
  const [hover, setHover] = createSignal(false);
  return <Pressable
    accessibilityLabel={props.row.name} role={props.onClick ? 1 : undefined}
    onClick={props.onClick} onHover={() => { setHover(true); props.onHover?.(); }}
    onLeave={() => { setHover(false); props.onLeave?.(); }}
    style={{ height: ROW, minWidth: 0, width: "100%", flexDirection: "row", alignItems: "center", gap: 8,
      paddingLeft: 10 + Math.min(props.row.depth, 16) * 12, paddingRight: 12,
      opacity: 1, backgroundColor: props.selected ? c().selected : hover() ? c().hover : "#00000000",
      cursor: props.onClick ? "pointer" : "default" }}>
    <Show when={props.row.expandable && props.onExpand}>
      <Pressable onClick={() => props.onExpand?.()} role={1} accessibilityLabel={props.expanded ? "Collapse" : "Expand"}
        style={{ width: 16, height: 22, alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
        <Label mono tone={c().accent}>{props.expanded ? "▾" : "▸"}</Label>
      </Pressable>
    </Show>
    <Row style={{ flex: 1, minWidth: 0, gap: 10, alignItems: "center" }}>
      <View style={{ minWidth: 0, flex: 1 }}><Label>{props.row.name}</Label></View>
      <Show when={props.row.kind}><Label mono subtle size={11}>{props.row.kind}</Label></Show>
      <Show when={props.row.detail}><View style={{ minWidth: 0, flex: 1 }}><Label subtle size={11}>{props.row.detail}</Label></View></Show>
    </Row>
    {props.children}
  </Pressable>;
}

export function useAsyncSlice<T>(load: (offset: number, limit: number) => Promise<T>, empty: T, report: (error: unknown) => void):
  { request: (offset: number, limit: number) => T; result: Accessor<T> } {
  const [bounds, setBounds] = createSignal<[number, number]>([0, 30]);
  const [result, setResult] = createSignal(empty);
  createTrackedEffect(() => {
    const [offset, limit] = bounds();
    let active = true;
    void load(offset, limit).then(value => { if (active) setResult(value); },
      error => { if (active) report(error); });
    onCleanup(() => { active = false; });
  });
  return { result, request: (offset, limit) => {
    const [a, b] = bounds();
    if (offset !== a || limit !== b) queueMicrotask(() => setBounds([offset, limit]));
    return result();
  } };
}
