import { For } from "solid-js";
import type { UIChildren, ViewStyle } from "@impasto/solid-renderer";
import { Column, Pressable, Role, Row, Text, View } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { settingsSig } from "../../state/settings.ts";
import { marketRunning, toggleMarket } from "../../state/quotes-provider.tsx";
import type { Sector } from "../../engine/analytics.ts";
import type { Quote } from "../../engine/types.ts";

export const colors = {
  bg: "#171A20", bar: "#1E2228", raised: "#32353B", line: "#30343B",
  ink: "#EEF0F4", dim: "#9DA2AA", faint: "#80858E",
  lime: "#C6EF71", up: "#43D3A0", down: "#FF6276", amber: "#FFB548", blue: "#7EAAFF",
};

export const filling: ViewStyle = { flex: 1, minHeight: 0, minWidth: 0, gap: 0 };

export function change(quote: Quote): number {
  return (quote.last - quote.prevClose) / quote.prevClose * 100;
}

export function sign(value: number, digits = 2): string {
  return `${value >= 0 ? "+" : ""}${value.toFixed(digits)}`;
}

export function tone(value: number): string {
  const reversed = settingsSig().updown === "cn";
  return value === 0 ? colors.dim : (value > 0) !== reversed ? colors.up : colors.down;
}

export function compact(value: number): string {
  const magnitude = Math.abs(value);
  return magnitude >= 1_000_000_000 ? `${(value / 1_000_000_000).toFixed(2)}B`
    : magnitude >= 1_000_000 ? `${(value / 1_000_000).toFixed(2)}M`
    : magnitude >= 10_000 ? `${(value / 1_000).toFixed(1)}K` : String(Math.round(value));
}

export function clock(ms: number, seconds = true): string {
  const sec = Math.max(0, Math.floor(ms / 1000));
  const part = (n: number) => String(n).padStart(2, "0");
  return `${part(Math.floor(sec / 3600) % 24)}:${part(Math.floor(sec / 60) % 60)}${seconds ? `:${part(sec % 60)}` : ""}`;
}

export function code(symbol: string): string {
  return symbol.split(".")[0]!;
}

export function sectorLabel(sector: Sector | undefined): string {
  switch (sector) {
    case "technology": return loc("科技", "Technology");
    case "health": return loc("医疗", "Health");
    case "finance": return loc("金融", "Finance");
    case "consumer": return loc("消费", "Consumer");
    case "industrials": return loc("工业", "Industrials");
    case "energy": return loc("能源", "Energy");
    default: return loc("其他", "Other");
  }
}

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
    <For each={props.options}>{(item) => <Pressable role={Role.Button} onClick={() => props.onSelect(item)}
      style={{ flexDirection: "row", gap: 4, alignItems: "center", justifyContent: "center", minHeight: 25, paddingLeft: 8, paddingRight: 8,
        borderRadius: 6, backgroundColor: props.selected === item ? colors.raised : "#00000000" }}>
      <Label size={12} color={props.selected === item ? colors.ink : colors.dim}>{props.label(item)}</Label>
      {() => props.count?.(item) === undefined ? undefined : <Label size={10} color={colors.faint}>{props.count?.(item)}</Label>}
    </Pressable>}</For>
  </Row>;
}

export function Legend(props: { color: string; label: string; value: string }) {
  return <Row style={{ gap: 5, alignItems: "center" }}>
    <View style={{ width: 8, height: 8, borderRadius: 2, backgroundColor: props.color }} />
    <Label size={11} color={colors.dim}>{props.label}</Label>
    <Label size={11}>{props.value}</Label>
  </Row>;
}
