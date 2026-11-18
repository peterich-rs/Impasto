import { For, Show } from "solid-js";
import type { UIChildren, ViewStyle } from "@impasto/solid-renderer";
import { Column, Pressable, Role, Row, Text, View } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { marketRunning, toggleMarket } from "../../state/quotes-provider.tsx";
import { colors } from "./theme.ts";

export { change, clock, code, colors, compact, filling, sectorLabel, sign, tone } from "./theme.ts";

export function Label(props: { children?: unknown; color?: string; size?: number; weight?: number; style?: ViewStyle }) {
  return <Text style={[{ color: props.color ?? colors.ink, fontSize: props.size ?? 13, fontWeight: props.weight ?? 400,
    maxLines: 1, textOverflow: "ellipsis", minWidth: 0 }, props.style]}>{props.children}</Text>;
}

export function Rule(props: { vertical?: boolean }) {
  return <View style={{ [props.vertical ? "width" : "height"]: 1, backgroundColor: colors.line, flexShrink: 0 }} />;
}

export function Section(props: { children?: UIChildren; title?: string; note?: string; action?: UIChildren }) {
  return <Column style={{ gap: 12, padding: 16, minWidth: 0 }}>
    {() => props.title ? <Row style={{ gap: 8, alignItems: "center", justifyContent: "space-between" }}>
      <Label size={15} weight={600} style={{ flex: 1 }}>{props.title}</Label>
      {props.action ?? <Label color={colors.faint} size={11}>{props.note ?? ""}</Label>}
    </Row> : undefined}
    {props.children}
  </Column>;
}

export function Action(props: { label: string; title?: string; primary?: boolean; onClick: () => void; style?: ViewStyle }) {
  return <Pressable role={Role.Button} accessibilityLabel={props.title ?? props.label} onClick={props.onClick}
    style={[{ minHeight: 32, paddingLeft: 10, paddingRight: 10, justifyContent: "center", alignItems: "center",
      borderRadius: 8, backgroundColor: props.primary ? colors.lime : colors.raised }, props.style]}>
    <Label weight={500} color={props.primary ? colors.bg : colors.ink}>{props.label}</Label>
  </Pressable>;
}

export function ClockButton() {
  return <Action label={marketRunning() ? "Ⅱ" : "▶"} title={marketRunning() ? loc("暂停行情", "Pause market") : loc("恢复行情", "Resume market")}
    primary={!marketRunning()} onClick={toggleMarket} style={{ width: 32, height: 32, padding: 0, borderRadius: 16 }} />;
}

export function Segments<T extends string>(props: {
  options: readonly T[]; selected: T; label: (item: T) => string; count?: (item: T) => number | undefined; onSelect: (item: T) => void;
}) {
  return <Row style={{ gap: 1, padding: 2, borderRadius: 9, borderWidth: 1, borderColor: colors.line, alignSelf: "start", maxWidth: "100%" }}>
    <For each={props.options}>{(item) => {
      const count = () => props.count?.(item);
      return <Pressable role={Role.Button} onClick={() => props.onSelect(item)}
        style={{ flexDirection: "row", gap: 4, alignItems: "center", justifyContent: "center", minHeight: 25, paddingLeft: 8, paddingRight: 8,
          borderRadius: 6, backgroundColor: props.selected === item ? colors.raised : "#00000000" }}>
        <Label size={12} color={props.selected === item ? colors.ink : colors.dim}>{props.label(item)}</Label>
        <Show when={count() !== undefined}><Label size={10} color={colors.faint}>{count()}</Label></Show>
      </Pressable>;
    }}</For>
  </Row>;
}

export function Legend(props: { color: string; label: string; value: string }) {
  return <Row style={{ gap: 5, alignItems: "center" }}>
    <View style={{ width: 8, height: 8, borderRadius: 2, backgroundColor: props.color }} />
    <Label size={11} color={colors.dim}>{props.label}</Label>
    <Label size={11}>{props.value}</Label>
  </Row>;
}
