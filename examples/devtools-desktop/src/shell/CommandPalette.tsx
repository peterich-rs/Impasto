import { For, Show, createMemo, createSignal } from "solid-js";
import { Column, Pressable, Row, ScrollArea } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";
import { Bar, Divider, Empty, Label, Search, Spacer, Tool } from "../components/ui";
import { pages } from "../state/navigation";

function matches(label: string, query: string) {
  const text = label.toLowerCase();
  let start = 0;
  for (const char of query.trim().toLowerCase()) {
    const next = text.indexOf(char, start);
    if (next < 0) return false;
    start = next + 1;
  }
  return true;
}

export function CommandPalette() {
  const ws = useWorkspace();
  const c = useTokens();
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  const commands = createMemo(() => [
    ...pages.map((page, index) => ({ label: `Go to ${page.label}`, hint: `Cmd ${index + 1}`, act: () => ws.nav.go({ kind: page.id }) })),
    ...ws.sessions().map(target => ({ label: `Attach to ${target.name}`, hint: `${target.platform} · ${target.id}`, act: () => void ws.attach(target.id) })),
    { label: "Open artifact", hint: "Cmd O", act: () => void ws.openArtifact() },
    { label: "Export session", hint: "Cmd E", act: () => void ws.saveSession() },
    { label: ws.drawer() ? "Hide console" : "Show console", hint: "Cmd J", act: () => ws.showDrawer(!ws.drawer()) },
    { label: ws.inspector() ? "Hide inspector" : "Show inspector", hint: "Cmd I", act: () => ws.showInspector(!ws.inspector()) },
    ...(["system", "dark", "light"] as const).map(theme => ({
      label: `Appearance: ${theme}`, hint: "", act: () => ws.setTheme(theme),
    })),
    ...(ws.live() ? [
      { label: "Disconnect target", hint: "", act: () => void ws.run(core.disconnect) },
      { label: ws.capture() ? "Stop trace recording" : "Record trace", hint: "", act: () => void ws.run(ws.record) },
      { label: "Collect JavaScript garbage", hint: "", act: () => void ws.run(async () => {
        await core.runJsGc(); ws.nav.go({ kind: "js", subTab: "memory" });
      }) },
      { label: "Pick a view on target", hint: "", act: () => void ws.updateOverlay({ pickMode: !ws.overlay().pickMode }) },
      { label: "Toggle repaint rainbow", hint: "", act: () => void ws.updateOverlay({ repaintRainbow: !ws.overlay().repaintRainbow }) },
      ...[
        ["off", "Off"], ["counters", "Counters"], ["gpu_timeline", "GPU timeline"], ["deep_inspect", "Deep Inspect"],
      ].map(([id, label]) => ({ label: `Telemetry: ${label}`, hint: "", act: () => void ws.run(() => core.setProfile(id)) })),
    ] : []),
  ].filter(command => matches(command.label, query())).slice(0, 40));
  function execute(index = selected()) {
    const target = commands()[index];
    if (!target) return;
    ws.setCommand(false);
    target.act();
  }
  function move(key: string): boolean {
    if (key !== "Up" && key !== "Down") return false;
    const delta = key === "Up" ? -1 : 1;
    setSelected(previous => Math.max(0, Math.min(commands().length - 1, previous + delta)));
    return true;
  }
  return <Column style={{ width: 480, maxHeight: 400, alignSelf: "center", backgroundColor: c().panel, borderWidth: 1,
    borderColor: c().strong, borderRadius: 8, overflow: "hidden" }}>
    <Bar>
      <Search value={query()} change={text => { setQuery(text); setSelected(0); }} width={398}
        placeholder="Type a command or target" controller={field => queueMicrotask(() => field.focus())}
        submit={() => execute()} onKeyDown={move} />
      <Spacer /><Tool label="Close commands" icon="close" iconOnly onClick={() => ws.setCommand(false)} />
    </Bar>
    <Divider />
    <Show when={commands().length > 0} fallback={<Empty title="No matching commands" />}>
      <ScrollArea style={{ height: Math.min(312, commands().length * 30), minHeight: 0 }}>
        <For each={commands()}>{(item, index) =>
          <Pressable role={1} accessibilityLabel={item.label} onClick={() => execute(index())}
            onHover={() => setSelected(index())}
            style={{ height: 30, paddingLeft: 12, paddingRight: 12, cursor: "pointer",
              backgroundColor: selected() === index() ? c().selected : c().panel }}>
            <Row style={{ flex: 1, alignItems: "center", gap: 8 }}>
              <Label>{item.label}</Label><Spacer /><Label mono subtle>{item.hint}</Label>
            </Row>
          </Pressable>
        }</For>
      </ScrollArea>
    </Show>
  </Column>;
}
