import { createContext, createMemo, createSignal, createTrackedEffect, onCleanup, untrack, useContext } from "solid-js";
import { pickFiles, saveFile } from "@impasto/file-system";
import { setString } from "@impasto/clipboard";
import type { FieldController } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { streamTicks } from "./core";
import { navigation, type NavigationIntent, type Page } from "./navigation";

export function createWorkspace() {
  const [notice, setNotice] = createSignal("");
  const [page, setPage] = createSignal<Page>("overview");
  const [intent, setIntent] = createSignal<NavigationIntent>({ kind: "overview" });
  const [consoleIntent, setConsoleIntent] = createSignal<Extract<NavigationIntent, { kind: "console" }>>();
  const [inspector, showInspector] = createSignal(true);
  const [inspectorWidth, setInspectorWidth] = createSignal(339);
  const [drawer, showDrawer] = createSignal(false);
  const [frameTab, setFrameTab] = createSignal("breakdown");
  const appearance = core.preferredAppearance();
  const [theme, setTheme] = createSignal<"system" | "dark" | "light">(appearance === "dark" || appearance === "light" ? appearance : "system");
  const [editor, setEditor] = createSignal<"vscode" | "cursor" | "zed">("vscode");
  const [command, setCommand] = createSignal(false);
  const [overlay, setOverlay] = createSignal<core.Overlay>({ pickMode: false, repaintRainbow: false, layoutScan: false });
  const [sampling, setSampling] = createSignal(false);
  const [recordDetail, setRecordDetail] = createSignal<"stages" | "detailed">("stages");
  const [artifact, setArtifact] = createSignal<core.ArtifactSummary>();
  const [busy, setBusy] = createSignal(false);
  const [search, bindSearch] = createSignal<FieldController>();
  const [frameStep, setFrameStep] = createSignal(0);
  const stream = streamTicks(error => setNotice(String(error)));
  const tick = stream.tick;
  const state = createMemo(() => tick()?.state ?? "idle");
  const live = createMemo(() => state() === "connected");
  const generation = createMemo(() => tick()?.generation ?? "");
  const capture = createMemo(() => tick()?.activeCapture ?? undefined);
  const sessions = createMemo(() => tick()?.targets ?? []);
  const current = createMemo(() => tick()?.targetId ?? undefined);

  async function run<T>(task: () => T | Promise<T>): Promise<T | undefined> {
    try { return await task(); }
    catch (error) { setNotice(String(error)); }
  }

  const nav = navigation(next => {
    if (next.kind === "console") {
      setConsoleIntent(next);
      showDrawer(true);
      return;
    }
    if (next.kind === "frame") setFrameTab(next.tab ?? "breakdown");
    if (next.kind === "view" && next.highlight && live()) void updateOverlay({ highlightNode: next.nodeId });
    if ((next.kind === "frame" && next.frameId) || (next.kind === "view" && next.nodeId) ||
        (next.kind === "network" && next.token)) showInspector(true);
    setIntent(next);
    setPage(next.kind);
  });

  async function attach(id: string) {
    if (busy()) return;
    setBusy(true);
    setNotice("");
    try { await run(() => core.attach(id)); }
    finally { setBusy(false); }
  }

  const preferred = core.preferredTarget();
  let attachedPreferred = false;
  createTrackedEffect(() => {
    const list = sessions();
    if (!attachedPreferred && preferred && list.some(item => item.id === preferred)) {
      attachedPreferred = true;
      untrack(() => { void attach(preferred); });
    }
  });

  createTrackedEffect(() => {
    if (state() === "reconnecting" || state() === "offline_snapshot") {
      setOverlay({ pickMode: false, repaintRainbow: false, layoutScan: false });
      setSampling(false);
    }
  });
  let lastGeneration = "";
  let lastPick = "";
  createTrackedEffect(() => {
    const next = generation();
    if (next === lastGeneration) return;
    lastGeneration = next;
    lastPick = "";
    setOverlay({ pickMode: false, repaintRainbow: false, layoutScan: false });
    setSampling(false);
  });
  createTrackedEffect(() => {
    const picked = tick()?.pickedNodeId;
    const seq = tick()?.pickedSeq;
    if (!picked || !seq || seq === lastPick) return;
    lastPick = seq;
    untrack(() => nav.go({ kind: "view", nodeId: picked, mode: "tree" }));
  });

  const leases = new Set<string>();
  const releasing = new Map<string, ReturnType<typeof setTimeout>>();
  function publishChannels() {
    void run(() => core.setActiveChannels([...leases]));
  }
  createTrackedEffect(() => {
    const active = page();
    const config = overlay();
    const wanted = new Set<string>();
    if (active === "overview" || active === "js" || active === "resources") wanted.add("runtime");
    if (active === "js" || active === "resources" || capture() || sampling()) wanted.add("resources");
    if (active === "view" || active === "frame" && frameTab() === "repaints" || config.pickMode || config.repaintRainbow || capture()) wanted.add("repaints");
    let changed = false;
    for (const channel of wanted) {
      const timer = releasing.get(channel);
      if (timer != null) { clearTimeout(timer); releasing.delete(channel); }
      if (!leases.has(channel)) { leases.add(channel); changed = true; }
    }
    for (const channel of leases) {
      if (!wanted.has(channel) && !releasing.has(channel)) {
        releasing.set(channel, setTimeout(() => {
          leases.delete(channel);
          releasing.delete(channel);
          publishChannels();
        }, 5000));
      }
    }
    if (changed) publishChannels();
  });
  onCleanup(() => { for (const timer of releasing.values()) clearTimeout(timer); });

  let overlayRequest = 0;
  async function updateOverlay(change: Partial<core.Overlay>) {
    const previous = overlay();
    const config = { ...previous, ...change };
    const request = ++overlayRequest;
    setOverlay(config);
    try { await core.setOverlay(config); }
    catch (error) {
      if (overlayRequest === request) setOverlay(previous);
      setNotice(String(error));
    }
  }

  async function openArtifact() {
    const [file] = await pickFiles({ accept: [".impasto-session", ".zip", ".pftrace", ".impsym", ".sym", ".map", ".json"] });
    if (!file) return;
    const loaded = await core.importArtifact(file.relativePath);
    setArtifact(loaded);
    if (loaded.kind === "Session recording") nav.go({ kind: "overview" });
    if (loaded.kind === "Bundle size report") nav.go({ kind: "resources", subTab: "bundle" });
    if (loaded.kind === "Perfetto trace") nav.go({ kind: "resources", subTab: "archive" });
    setNotice(`Opened ${loaded.name}`);
  }

  async function save(stage: Promise<string>, name: string, extension: string) {
    const path = await stage;
    try {
      if (await saveFile({ sourcePath: path, scope: "cache", suggestedName: name, allowedExtensions: [extension] })) {
        setNotice(`Saved ${name}`);
      }
    } finally { await core.removeStaged(path); }
  }

  async function record() {
    if (capture()) await core.traceStop();
    else await core.traceStart(recordDetail(), true, recordDetail() === "detailed");
  }

  function openSource(path: string, line: number, column: number) {
    void run(() => core.openSourceInEditor(path, line, column, editor()));
  }

  return {
    ...stream, tick, live, state, generation, current, sessions, capture,
    notice, setNotice, page, intent, consoleIntent, nav, inspector, showInspector, drawer, showDrawer,
    inspectorWidth, setInspectorWidth, frameTab, setFrameTab, theme, setTheme, editor, setEditor, command, setCommand,
    overlay, updateOverlay, sampling, setSampling, recordDetail, setRecordDetail, record,
    busy, attach, artifact, bindSearch, focusSearch: () => search()?.focus(), frameStep,
    stepFrame: (delta: number) => setFrameStep(value => value + delta), run, openSource,
    openArtifact: () => run(openArtifact),
    saveTrace: (id: string) => run(() => save(core.stagePftraceExport(id), "capture.pftrace", "pftrace")),
    saveSession: (format: "zip" | "gzip" = "zip") => run(() => save(core.stageSessionExport(format),
      format === "zip" ? "session.impasto-devtools.zip" : "capture.impasto-session", format === "zip" ? "zip" : "impasto-session")),
    copy: (text: string) => run(() => { setString(text); setNotice("Copied to clipboard"); }),
    togglePause: () => run(() => core.setPaused(!tick()?.paused)),
  };
}

export const Workspace = createContext<ReturnType<typeof createWorkspace>>();
export function useWorkspace() {
  const state = useContext(Workspace);
  if (!state) throw new Error("DevTools workspace is missing");
  return state;
}
