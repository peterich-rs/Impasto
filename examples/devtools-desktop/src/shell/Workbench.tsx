import { For, Show, createMemo, createSignal, createTrackedEffect } from "solid-js";
import { Column, Pressable, Row, ScrollArea, View } from "@impasto/solid-ui";
import { scene } from "@impasto/solid-renderer";
import * as core from "@app/devtools-core";
import { Bar, Divider, Empty, Label, Spacer, Tool, ms } from "../components/ui";
import { Icon } from "../components/Icons";
import { SplitHandle } from "../components/SplitPane";
import { pages } from "../state/navigation";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";
import { CommandPalette } from "./CommandPalette";
import { ConsoleDrawer } from "./ConsoleDrawer";
import { OverviewPage } from "../pages/OverviewPage";
import { FramesPage } from "../pages/FramesPage";
import { ViewsPage } from "../pages/ViewsPage";
import { JsRuntimePage } from "../pages/JsRuntimePage";
import { NetworkPage } from "../pages/NetworkPage";
import { ResourcesPage } from "../pages/ResourcesPage";

const profiles = [{ id: "off", label: "Off" }, { id: "counters", label: "Counters" },
  { id: "gpu_timeline", label: "GPU timeline" }, { id: "deep_inspect", label: "Deep Inspect" }];

export function Workbench() {
  const ws = useWorkspace();
  const c = useTokens();
  const [targets, showTargets] = createSignal(false);
  const [consoleHeight, setConsoleHeight] = createSignal(225);
  const state = ws.state;
  const hasSession = createMemo(() => !!ws.tick()?.session);
  const name = createMemo(() => ws.tick()?.session?.appName || "");
  const activeProfile = createMemo(() => profiles.find(item => item.id === ws.tick()?.profile) ?? profiles[1]);
  const connection = () => state() === "connected" ? ws.tick()?.paused ? "Paused" : "Live"
    : state() === "connecting" ? "Connecting" : state() === "reconnecting" ? "Reconnecting"
    : state() === "offline_snapshot" ? "Offline playback" : "No session";
  const color = () => state() === "connected" ? ws.tick()?.paused ? c().warn : c().ok
    : state() === "reconnecting" || state() === "offline_snapshot" ? c().warn : c().muted;
  const toggleCommands = () => { showTargets(false); ws.setCommand(!ws.command()); };
  const shortcut = (key: string, action: () => void) => scene.onShortcut(key, action);
  shortcut("mod+k", toggleCommands);
  shortcut("mod+f", () => ws.focusSearch());
  shortcut("mod+o", () => void ws.openArtifact());
  shortcut("mod+e", () => { if (hasSession()) void ws.saveSession(); });
  shortcut("mod+j", () => ws.showDrawer(!ws.drawer()));
  shortcut("mod+i", () => ws.showInspector(!ws.inspector()));
  shortcut("mod+[", () => ws.nav.step(-1));
  shortcut("mod+]", () => ws.nav.step(1));
  for (const [index, page] of pages.entries()) shortcut(`mod+${index + 1}`, () => ws.nav.go({ kind: page.id }));
  shortcut("escape", () => {
    if (ws.command()) ws.setCommand(false);
    else if (targets()) showTargets(false);
    else if (ws.overlay().pickMode && ws.live()) void ws.updateOverlay({ pickMode: false });
    else ws.setNotice("");
  });
  createTrackedEffect(() => { if (ws.busy()) showTargets(false); });

  return <Column style={{ width: "100%", height: "100%", minHeight: 0, backgroundColor: c().canvas, position: "relative" }}
    focusable
    onKey={key => {
      if (ws.command()) return;
      if ((key === " " || key === "Space") && ws.live()) void ws.togglePause();
      if (ws.page() === "frame" && key === "Left") ws.stepFrame(-1);
      if (ws.page() === "frame" && key === "Right") ws.stepFrame(1);
    }}>
    <Row style={{ height: 46, paddingLeft: 12, paddingRight: 10, flexShrink: 0, backgroundColor: c().panel, alignItems: "center", gap: 8 }}>
      <Row style={{ alignItems: "center", gap: 7, flexShrink: 0 }}>
        <Icon name="overview" color={c().accent} size={19} />
        <Label size={14} weight={600}>Impasto DevTools</Label>
      </Row>
      <View style={{ height: 20, width: 1, backgroundColor: c().border }} />
      <Tool label={name() || "Select target"} icon="target" active={targets()} disabled={ws.busy()}
        onClick={() => { ws.setCommand(false); showTargets(!targets()); }} />
      <Show when={ws.live()}>
        <Tool label={ws.tick()?.paused ? "Resume" : "Pause"} icon={ws.tick()?.paused ? "play" : "pause"} onClick={() => void ws.togglePause()} />
        <Tool label={ws.capture() ? "Stop" : "Record"} icon={ws.capture() ? "stop" : "record"}
          tone={ws.capture() ? c().danger : undefined} active={!!ws.capture()} onClick={() => void ws.run(ws.record)} />
        <Tool label={activeProfile().label} onClick={() => {
          const next = profiles[(profiles.indexOf(activeProfile()) + 1) % profiles.length];
          void ws.run(() => core.setProfile(next.id));
        }} />
      </Show>
      <Spacer />
      <Tool label="Open artifact" icon="open" iconOnly onClick={() => void ws.openArtifact()} />
      <Tool label="Export session" icon="export" iconOnly disabled={!hasSession()} onClick={() => void ws.saveSession()} />
      <Tool label="Inspector" icon="panel" iconOnly active={ws.inspector()} onClick={() => ws.showInspector(!ws.inspector())} />
      <Tool label="Cmd K" icon="search" active={ws.command()} onClick={toggleCommands} />
    </Row>
    <Divider />
    <Show when={state() === "reconnecting" || state() === "offline_snapshot" || ws.tick()?.reconnectedNewPid}>
      <View style={{ backgroundColor: c().elevated, flexShrink: 0 }}>
        <Bar>
          <Icon name="warning" color={c().warn} />
          <Label tone={c().warn}>{state() === "reconnecting" ? `Reconnecting · attempt ${ws.tick()?.reconnectAttempt} · viewing last snapshot`
            : state() === "offline_snapshot" ? "Read-only snapshot" : "Attached to a new target process"}</Label>
          <Spacer />
          <Show when={!ws.live() && ws.current()}>
            <Tool label="Reconnect now" onClick={() => void ws.run(core.reconnectNow)} />
          </Show>
        </Bar>
        <Divider />
      </View>
    </Show>

    <Row style={{ flex: 1, minHeight: 0, minWidth: 0 }}>
      <Column style={{ width: 72, flexShrink: 0, backgroundColor: c().panel, paddingTop: 6, alignItems: "center", gap: 3 }}>
        <For each={pages}>{page => {
          const [hover, setHover] = createSignal(false);
          const active = () => ws.page() === page.id;
          return <Pressable role={1} accessibilityLabel={page.label} onClick={() => ws.nav.go({ kind: page.id })}
            onHover={() => setHover(true)} onLeave={() => setHover(false)}
            style={{ width: "100%", height: 48, alignItems: "center", justifyContent: "center", gap: 4,
              position: "relative", backgroundColor: active() ? c().selected : hover() ? c().hover : c().panel, cursor: "pointer" }}>
            <View style={{ position: "absolute", left: 0, top: 6, bottom: 6, width: 2, backgroundColor: active() ? c().accent : "#00000000" }} />
            <Icon name={page.id} size={18} color={active() ? c().accent : c().secondary} />
            <Label size={9} tone={active() ? c().accent : c().secondary}>{page.label}</Label>
          </Pressable>;
        }}</For>
        <Spacer />
        <Tool label="Console" icon="terminal" active={ws.drawer()} iconOnly onClick={() => ws.showDrawer(!ws.drawer())} />
        <Show when={(ws.tick()?.errorCount ?? 0) > 0}>
          <Tool label={`${ws.tick()?.errorCount} errors`} tone={c().danger} onClick={() => ws.nav.go({ kind: "console", errorSeq: ws.tick()?.latestErrorSeq ?? undefined })} />
        </Show>
        <View style={{ height: 4 }} />
      </Column>
      <Divider vertical />
        <Show when={ws.page() === "overview"}><OverviewPage /></Show>
        <Show when={ws.page() === "frame"}><FramesPage /></Show>
        <Show when={ws.page() === "view"}><ViewsPage /></Show>
        <Show when={ws.page() === "js"}><JsRuntimePage /></Show>
        <Show when={ws.page() === "network"}><NetworkPage /></Show>
        <Show when={ws.page() === "resources"}><ResourcesPage /></Show>
    </Row>
    <Show when={ws.drawer()}>
      <SplitHandle resize={delta => setConsoleHeight(height => Math.max(110, Math.min(360, height + delta)))} />
      <ConsoleDrawer height={consoleHeight()} />
    </Show>
    <Divider />
    <Row style={{ height: 27, flexShrink: 0, paddingLeft: 8, paddingRight: 9, backgroundColor: c().panel, alignItems: "center", gap: 9 }}>
      <Tool label="Back" icon="left" iconOnly disabled={ws.nav.position().index === 0} onClick={() => ws.nav.step(-1)} />
      <Tool label="Forward" icon="chevron" iconOnly disabled={ws.nav.position().index + 1 === ws.nav.position().total} onClick={() => ws.nav.step(1)} />
      <View style={{ width: 6, height: 6, borderRadius: 3, flexShrink: 0, backgroundColor: color() }} />
      <Label size={11} tone={color()}>{connection()}</Label>
      <Show when={ws.tick()?.rttMs != null}><Label mono subtle size={11}>{ms(ws.tick()?.rttMs)} RTT</Label></Show>
      <View style={{ flex: 1, minWidth: 0 }}><Label subtle size={11}>{ws.notice() || ws.tick()?.message || ""}</Label></View>
      <Show when={hasSession()}>
        <Label mono subtle size={11}>{ws.tick()?.stats.fps.toFixed(0)} fps</Label>
        <Label mono subtle size={11}>P95 {ms(ws.tick()?.stats.p95Ms)}</Label>
        <Label mono subtle size={11}>{ws.tick()?.frameCount} frames · {ws.tick()?.logCount} logs</Label>
      </Show>
    </Row>

    <Show when={targets()}>
      <Pressable onClick={() => showTargets(false)} accessibilityLabel="Close target selector"
        style={{ position: "absolute", top: 46, bottom: 0, left: 0, right: 0, backgroundColor: "#00000018", zIndex: 8 }} />
      <Column style={{ position: "absolute", zIndex: 9, top: 45, left: 154, width: 380,
        maxHeight: 340, backgroundColor: c().panel, borderWidth: 1, borderRadius: 6, borderColor: c().strong, overflow: "hidden" }}>
        <Bar><Label weight={600}>Available targets</Label><Spacer />
          <Show when={hasSession()}><Tool label="Disconnect" onClick={() => { showTargets(false); void ws.run(core.disconnect); }} /></Show>
        </Bar>
        <Divider />
        <Show when={ws.sessions().length > 0} fallback={<Empty title="No running targets" hint="Start a local app with DevTools enabled, or attach an Android or iOS device using the CLI." />}>
          <ScrollArea style={{ height: Math.min(ws.sessions().length * 34, 280), minHeight: 0 }}>
            <For each={ws.sessions()}>{target => <Pressable role={1} accessibilityLabel={target.name}
              onClick={() => { showTargets(false); void ws.attach(target.id); }}
              style={{ height: 34, flexDirection: "row", alignItems: "center", gap: 8, paddingLeft: 12, paddingRight: 12,
                backgroundColor: target.id === ws.current() ? c().selected : c().panel, cursor: "pointer" }}>
              <Icon name="target" /><Label>{target.name}</Label><Spacer />
              <Label mono subtle size={10}>{target.platform} · {target.id}</Label>
              <Show when={target.hasSourceMap}><Label subtle size={10}>Symbols</Label></Show>
            </Pressable>}</For>
          </ScrollArea>
        </Show>
      </Column>
    </Show>
    <Show when={ws.command()}>
      <View style={{ position: "absolute", inset: 0, zIndex: 12, alignItems: "center", paddingTop: 76 }}>
        <Pressable onClick={() => ws.setCommand(false)} accessibilityLabel="Dismiss command palette"
          style={{ position: "absolute", inset: 0, backgroundColor: "#00000060" }} />
        <CommandPalette />
      </View>
    </Show>
  </Column>;
}
