import { createMemo, For, Show } from "solid-js";
import type { NodeRef, UIChildren, ViewStyle } from "@impasto/solid-renderer";
import { Column, Pressable, Role, Row, Text, View, useTheme } from "@impasto/solid-ui";
import { loc } from "../../copy.ts";
import { marketRunning, toggleMarket } from "../../state/quotes-provider.tsx";
import { colors, tone } from "./theme.ts";
import { Glyph } from "./icons.tsx";

export { change, clock, code, colors, compact, filling, sectorLabel, sign, tone } from "./theme.ts";
export { BackButton, Glyph, Icon, type IconName } from "./icons.tsx";

export function Label(props: { ref?: NodeRef; children?: unknown; color?: string; size?: number; weight?: number; style?: ViewStyle }) {
  return <Text ref={props.ref} style={[{ color: props.color ?? colors.ink, fontSize: props.size ?? 13, fontWeight: props.weight ?? 400,
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

export function Action(props: { label?: string; icon?: UIChildren; title?: string; primary?: boolean; onClick: () => void; style?: ViewStyle }) {
  return <Pressable role={Role.Button} accessibilityLabel={props.title ?? props.label} onClick={props.onClick}
    style={[{ minHeight: 36, minWidth: 36, paddingLeft: 12, paddingRight: 12, paddingTop: 0, paddingBottom: 0, justifyContent: "center", alignItems: "center",
      flexDirection: "row", gap: 6, borderRadius: 8, backgroundColor: props.primary ? colors.lime : colors.raised }, props.style]}>
    {props.icon}
    <Show when={props.label !== undefined}><Label weight={500} color={props.primary ? colors.bg : colors.ink}>{props.label}</Label></Show>
  </Pressable>;
}

export function ClockButton() {
  const running = marketRunning();
  return <Glyph name={running ? "pause" : "play"} title={running ? loc("暂停行情", "Pause market") : loc("恢复行情", "Resume market")}
    primary={!running} onClick={toggleMarket} style={{ width: 36, height: 36, minWidth: 36, minHeight: 36, borderRadius: 18 }} />;
}

export function Segments<T extends string | number>(props: {
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

export function useLegendNumberStyle(): () => ViewStyle {
  const theme = useTheme();
  return createMemo(() => ({
    width: Math.ceil(56 * theme().textScale),
    height: Math.ceil(16 * theme().textScale),
    textAlign: "left",
  }));
}

export function Legend(props: { color: string; label: string; value: string; valueStyle?: ViewStyle }) {
  const theme = useTheme();
  const swatch = createMemo(() => {
    const size = 8 * theme().textScale;
    return { width: size, height: size, borderRadius: size / 4 };
  });
  return <Row style={{ gap: 5, alignItems: "baseline", lineHeight: 1.2 }}>
    <View style={{ ...swatch(), backgroundColor: props.color }} />
    <Label size={11} color={colors.dim}>{props.label}</Label>
    <Label size={11} style={props.valueStyle}>{props.value}</Label>
  </Row>;
}

/**
 * Price in a tone-colored badge. `change` is the signed percent; the face
 * follows the up/down convention from settings. Width is clamped so digits
 * ticking in and out never relayout the row.
 */
export function PriceTag(props: { value: string; change: number; size?: number; width?: number }) {
  return <View style={{ minWidth: props.width ?? 76, height: 28, paddingLeft: 8, paddingRight: 8, borderRadius: 6, alignItems: "center", justifyContent: "center",
    backgroundColor: tone(props.change) }}>
    <Label size={props.size ?? 15} weight={600} color={colors.bg}>{props.value}</Label>
  </View>;
}
