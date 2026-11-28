import { For, Show, createMemo, createSignal, createTrackedEffect } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { SliceTable, TableRow } from "../components/SliceTable";
import { SourceLink } from "../components/SourceLink";
import { Bar, Divider, KeyValues, Label, Pane, Search, Spacer, Tabs, Tool } from "../components/ui";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";

const modes = [{ id: "logs", label: "Logs" }, { id: "errors", label: "Errors" }];
const levels = ["", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"];
const sources = ["", "script", "layout", "host", "net"];

export function ConsoleDrawer(props: { full?: boolean; height?: number }) {
  const ws = useWorkspace();
  const c = useTokens();
  const [tab, setTab] = createSignal("logs");
  const [level, setLevel] = createSignal("");
  const [source, setSource] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [frame, setFrame] = createSignal("");
  const [selected, select] = createSignal<core.Row>();
  const [raw, setRaw] = createSignal(false);
  createTrackedEffect(() => {
    const intent = ws.consoleIntent();
    if (!intent) return;
    setFrame(intent.frameId ?? "");
    if (intent.errorSeq) {
      setTab("errors");
      select({ id: intent.errorSeq, name: "Error", kind: "", detail: "", meta: intent.frameId ?? "",
        depth: 0, expandable: false, atMs: 0, count: 0 });
    }
  });
  const errors = () => tab() === "errors";
  const detail = createMemo(() => {
    const row = selected();
    if (!row) return [];
    return core.consoleDetail(row.id, errors(), raw()).filter(field => field.name !== "stack");
  });
  const stack = createMemo(() => {
    const row = selected();
    if (!row || !errors()) return "";
    ws.error();
    return core.consoleStack(row.id, raw()) ?? "";
  });
  return <Column style={{ height: props.height, flex: props.full ? 1 : undefined, minHeight: 0,
    backgroundColor: c().panel, overflow: "hidden" }}>
    <Bar>
      <Tabs items={modes} selected={tab()} select={value => { setTab(value); select(undefined); setRaw(false); }} />
      <Search value={search()} change={setSearch} placeholder="Search console" width={185} controller={ws.bindSearch} />
      <Show when={!errors()}>
        <Tool label={level() || "All levels"} onClick={() => setLevel(levels[(levels.indexOf(level()) + 1) % levels.length])} />
      </Show>
      <Tool label={source() || "All sources"} onClick={() => setSource(sources[(sources.indexOf(source()) + 1) % sources.length])} />
      <Show when={frame()}><Tool label={`Frame #${frame()} ×`} active onClick={() => setFrame("")} /></Show>
      <Spacer />
      <Show when={!props.full}><Tool label="Close console" icon="close" iconOnly onClick={() => ws.showDrawer(false)} /></Show>
      <Tool label="Clear" disabled={!ws.live()} onClick={() => { void ws.run(() => core.clearSession("logs")); select(undefined); }} />
    </Bar>
    <Divider />
    <Row style={{ flex: 1, minHeight: 0, minWidth: 0 }}>
      <SliceTable reset={`${tab()}|${level()}|${source()}|${search()}|${frame()}`} follow={!errors()} query={(offset, limit) => {
        if (errors()) { ws.error(); return core.queryErrorsSlice(search(), source(), frame(), offset, limit); }
        ws.log(); return core.queryLogsSlice(level(), source(), search(), frame(), offset, limit);
      }} render={row => <TableRow row={row()} selected={selected()?.id === row().id} onClick={() => select(row())}>
        <Show when={row().meta}><Tool label={`#${row().meta}`} onClick={() => ws.nav.go({ kind: "frame", frameId: row().meta })} /></Show>
        <Label mono subtle size={10}>{(row().atMs / 1000).toFixed(3)}</Label>
      </TableRow>} />
      <Show when={selected()}>{row => <>
        <Divider vertical />
        <Column style={{ width: 320, minHeight: 0 }}>
          <Bar><Label weight={600}>{errors() ? "Exception" : "Log entry"}</Label><Spacer />
            <Tool label="Close details" icon="close" iconOnly onClick={() => select(undefined)} />
          </Bar>
          <Show when={errors() && stack()}>
            <Bar>
              <Tool label={raw() ? "Show symbolicated" : "Show raw"} onClick={() => setRaw(!raw())} />
              <Tool label="Copy stack" onClick={() => void ws.copy(stack())} />
            </Bar>
          </Show>
          <Pane><View style={{ padding: 10 }}>
            <KeyValues fields={detail()} />
            <For each={stack().split("\n").filter(Boolean)}>{line => <SourceLink text={line} />}</For>
            <Show when={row().meta}>
              <Tool label={`Inspect frame #${row().meta}`} onClick={() => ws.nav.go({ kind: "frame", frameId: row().meta })} />
            </Show>
          </View></Pane>
        </Column>
      </>}</Show>
    </Row>
  </Column>;
}
