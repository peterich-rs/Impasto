import { For, Show, createSignal } from "solid-js";
import { Column, Field, Pressable, Row, ScrollArea, Text, View, type UIChildren } from "@impasto/solid-ui";
import type { FieldController } from "@impasto/solid-ui";
import type { DetailField } from "@app/devtools-core";
import { useTokens, mono } from "../theme/tokens";
import { Icon, type Glyph } from "./Icons";

export function Divider(props: { vertical?: boolean }) {
  const c = useTokens();
  return <View style={{ width: props.vertical ? 1 : "100%", height: props.vertical ? "100%" : 1, flexShrink: 0, backgroundColor: c().border }} />;
}

export function Label(props: { children?: UIChildren; subtle?: boolean; size?: number; mono?: boolean; tone?: string; lines?: number; weight?: number }) {
  const c = useTokens();
  return <Text
    style={{ color: props.tone ?? (props.subtle ? c().secondary : c().primary),
      fontSize: props.size ?? 12, fontWeight: props.weight ?? 400, ...(props.mono ? mono : {}), maxLines: props.lines ?? 1, textOverflow: "ellipsis" }}>
    {props.children}
  </Text>;
}

export function Tool(props: { label: string; icon?: Glyph; active?: boolean; disabled?: boolean; iconOnly?: boolean; tone?: string; onClick: () => void }) {
  const c = useTokens();
  const [hover, setHover] = createSignal(false);
  return <Pressable
    disabled={props.disabled}
    accessibilityLabel={props.label}
    role={1}
    style={{
      minWidth: 28, height: 28, flexShrink: 0, paddingLeft: props.iconOnly ? 6 : 8, paddingRight: props.iconOnly ? 6 : 8,
      alignItems: "center", justifyContent: "center", flexDirection: "row", gap: 6, borderRadius: 4,
      backgroundColor: props.active ? c().selected : hover() ? c().hover : "#00000000",
      opacity: props.disabled ? 0.45 : 1, cursor: props.disabled ? "default" : "pointer",
    }}
    onHover={() => setHover(true)} onLeave={() => setHover(false)} onClick={props.onClick}>
    <Show when={props.icon}>{icon => <Icon name={icon()} color={props.tone ?? (props.active ? c().accent : undefined)} />}</Show>
    <Show when={!props.iconOnly}><Label tone={props.tone ?? (props.active ? c().accent : undefined)}>{props.label}</Label></Show>
  </Pressable>;
}

export function Tabs<T extends string>(props: { items: readonly { id: T; label: string }[]; selected: T; select: (id: T) => void }) {
  return <Row style={{ gap: 4, flexWrap: "wrap" }}>
    <For each={props.items}>{item => <Tool label={item.label} active={props.selected === item.id} onClick={() => props.select(item.id)} />}</For>
  </Row>;
}

export function Search(props: { value: string; change: (value: string) => void; placeholder: string; width?: number; controller?: (field: FieldController) => void; submit?: (value: string) => void; onKeyDown?: (key: string) => boolean }) {
  const c = useTokens();
  return <Field
    style={{ width: props.width ?? 220, minWidth: 100 }}
    controlProps={{ style: { height: 28, minHeight: 28, paddingLeft: 8, paddingRight: 8, paddingTop: 0, paddingBottom: 0, backgroundColor: c().canvas, borderColor: c().border, borderWidth: 1, borderRadius: 4 } }}
    textProps={{ style: { fontSize: 12, color: c().primary } }}
    value={props.value}
    placeholder={props.placeholder}
    maxLength={200}
    onChange={props.change}
    onSubmit={props.submit}
    onKeyDown={props.onKeyDown}
    controller={props.controller}
  />;
}

export function Bar(props: { children?: UIChildren }) {
  return <Row style={{ alignItems: "center", minHeight: 36, paddingLeft: 10, paddingRight: 10, gap: 6, flexWrap: "wrap", flexShrink: 0 }}>{props.children}</Row>;
}

export function Section(props: { title: string; detail?: string; children?: UIChildren }) {
  return <Column style={{ padding: 12, gap: 8, flexShrink: 0 }}>
    <Row style={{ gap: 8, alignItems: "center", justifyContent: "space-between" }}>
      <Label weight={600} size={13}>{props.title}</Label>
      <Show when={props.detail}><Label subtle mono size={11}>{props.detail}</Label></Show>
    </Row>
    {props.children}
  </Column>;
}

export function Empty(props: { title: string; hint?: string; action?: UIChildren }) {
  return <Column style={{ padding: 20, gap: 8, alignItems: "flex-start", justifyContent: "center", minHeight: 88 }}>
    <Label size={14} weight={600}>{props.title}</Label>
    <Show when={props.hint}><Label subtle lines={3}>{props.hint}</Label></Show>
    {props.action}
  </Column>;
}

export function KeyValues(props: { fields: readonly DetailField[]; empty?: string }) {
  const c = useTokens();
  return <Show when={props.fields.length > 0} fallback={<Label subtle>{props.empty ?? "No retained details"}</Label>}>
    <For each={props.fields} keyed={field => `${field.depth}:${field.name}`}>{field => <Row style={{ gap: 8, paddingTop: 3, paddingBottom: 3, minHeight: 23, alignItems: "flex-start", justifyContent: "space-between", borderRadius: 2 }}>
      <Text style={{ color: c().secondary, fontSize: 11, paddingLeft: Math.min(field().depth, 3) * 5, width: "43%", flexShrink: 0, maxLines: 2 }}>{field().name}</Text>
      <Text style={{ ...mono, fontSize: 11, color: c().primary, textAlign: "right", flex: 1, minWidth: 0, maxLines: 6 }}>{field().value}</Text>
    </Row>}</For>
  </Show>;
}

export function Pane(props: { children?: UIChildren }) {
  return <ScrollArea style={{ flex: 1, minWidth: 0, minHeight: 0 }} contentProps={{ style: { width: "100%" } }}>
    {props.children}
  </ScrollArea>;
}

export function Spacer() { return <View style={{ flex: 1, minWidth: 0 }} />; }

export const bytes = (value: number) => value >= 1048576 ? `${(value / 1048576).toFixed(2)} MiB`
  : value >= 1024 ? `${(value / 1024).toFixed(1)} KiB` : `${Math.round(value)} B`;
export const ms = (value: number | null | undefined) => value == null ? "—" : `${value.toFixed(2)} ms`;
