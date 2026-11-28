import { For, Show, createMemo, createSignal, createTrackedEffect, untrack } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { InteractiveCanvas, type PlotSize } from "../charts/InteractiveCanvas";
import { Legend } from "../charts/Tracks";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Search, Section, Spacer, Tabs, Tool, bytes } from "../components/ui";
import { TableRow, SliceTable, useAsyncSlice } from "../components/SliceTable";
import { SourceLink } from "../components/SourceLink";
import { ConsoleDrawer } from "../shell/ConsoleDrawer";
import { PageLayout } from "../shell/PageLayout";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";

const tabs = [{ id: "flamegraph", label: "CPU profile" }, { id: "memory", label: "Memory & GC" },
  { id: "resources", label: "Timers & promises" }, { id: "console", label: "Console & errors" }];
const modes = [{ id: "flame", label: "Flamegraph" }, { id: "top_down", label: "Top-down" }, { id: "bottom_up", label: "Bottom-up" }];
const heap: readonly [string, number][] = [["Objects", 48], ["Strings", 49], ["Atoms", 50], ["Shapes", 51], ["Functions", 52], ["Binary", 53], ["Other", 54]];

function ProfileTable(props: {
  mode: string; search: string; frame: string | null; selected?: string; pick: (row: core.Row) => void;
}) {
  const ws = useWorkspace();
  const { request } = useAsyncSlice((offset, limit) => {
    ws.resource();
    return core.queryProfileTable(props.mode, props.search, props.frame, offset, limit);
  }, { total: 0, rows: [] }, error => ws.setNotice(String(error)));
  return <SliceTable query={request} empty="No sampled functions" reset={`${props.mode}|${props.search}`} render={row =>
    <TableRow row={row()} selected={row().name === props.selected} onClick={() => props.pick(row())} />} />;
}

export function JsRuntimePage() {
  const ws = useWorkspace();
  const c = useTokens();
  const [tab, setTab] = createSignal("flamegraph");
  const [mode, setMode] = createSignal("flame");
  const [query, setQuery] = createSignal("");
  const [frame, setFrame] = createSignal<string>();
  const [interval, setIntervalUs] = createSignal(1000);
  const [zoom, setZoom] = createSignal(0);
  const [size, setSize] = createSignal<PlotSize>({ width: 0, height: 0 });
  const [plot, setPlot] = createSignal<core.ProjectedCanvas>();
  const [hit, setHit] = createSignal<core.Hit>();
  const [tip, setTip] = createSignal("");
  const [gc, setGc] = createSignal<core.DetailField[]>();
  let picked = 0;
  createTrackedEffect(() => {
    const intent = ws.intent();
    if (intent.kind !== "js") return;
    setTab(intent.subTab ?? "flamegraph");
    setFrame(intent.frameId);
    setQuery(intent.search ?? "");
    setZoom(0);
  });
  createTrackedEffect(() => {
    ws.generation();
    if (ws.live()) untrack(() => { void ws.run(core.refreshJsRuntime); });
  });
  const capture = createMemo(() => ws.tick()?.latestCapture);
  createTrackedEffect(() => {
    const id = frame();
    const trace = capture();
    if (id && trace && ws.live() && core.captureContains(id)) void ws.run(() => core.loadFrameSpans(trace, id));
  });
  createTrackedEffect(() => {
    if (tab() !== "flamegraph" || mode() !== "flame" || !size().width) return;
    ws.resource();
    if (frame()) ws.frame();
    const request = { ...size(), frame: frame() ?? null, zoom: zoom(), query: query() };
    let active = true;
    const timer = setTimeout(() => void core.projectFlamegraph(request.width, request.height, request.frame, request.zoom, request.query)
      .then(value => { if (active) setPlot(value); }, error => { if (active) ws.setNotice(String(error)); }), 20);
    return () => { active = false; clearTimeout(timer); };
  });

  async function sampling(next: boolean, period = interval()) {
    await core.setJsSampler(next, period, 32, 160);
    ws.setSampling(next);
    setIntervalUs(period);
  }

  function inspect(point: PlotSize & { x: number; y: number }, select: boolean) {
    const request = ++picked;
    void core.hitTestFlamegraph(point.x, point.y, point.width, point.height, frame() ?? null, zoom(), query()).then(value => {
      if (request !== picked && !select) return;
      if (!select) { setTip(value?.detail ?? ""); return; }
      setHit(value ?? undefined);
      if (value) { setZoom(Number(value.id)); ws.showInspector(true); }
    }, error => ws.setNotice(String(error)));
  }

  const heapDetail = createMemo(() => { ws.runtime(); return core.jsMemoryDetail(); });
  const currentMemory = (name: string) => Number(heapDetail().find(field => field.name === name)?.value) || 0;
  return <PageLayout title="JS Runtime" inspectorTitle="Runtime inspector"
    actions={<>
      <Tool label={ws.sampling() ? "Stop sampling" : "Start sampling"} active={ws.sampling()} disabled={!ws.live()} onClick={() => void ws.run(() => sampling(!ws.sampling()))} />
      <Tool label={`${interval()} µs`} disabled={!ws.live()} onClick={() => {
        const next = interval() === 500 ? 1000 : interval() === 1000 ? 2000 : 500;
        if (ws.sampling()) void ws.run(() => sampling(true, next)); else setIntervalUs(next);
      }} />
      <Tool label="Run GC" disabled={!ws.live()} onClick={() => void ws.run(async () => { setGc(await core.runJsGc()); ws.showInspector(true); })} />
      <Tool label={ws.tick()?.symbolsLoaded ? "Symbols loaded" : "Load symbols"} icon="open" onClick={() => void ws.openArtifact()} />
    </>}
    inspector={<Pane>
      <Show when={hit()}>{selected => <><Section title="Selected function">
        <SourceLink text={selected().name} />
        <Label subtle mono>{selected().detail}</Label>
        <KeyValues fields={selected().fields} />
      </Section><Divider /></>}</Show>
      <Show when={gc()}>{selected => <><Section title="GC comparison">
        <KeyValues fields={selected()} /><Tool label="Close comparison" onClick={() => setGc(undefined)} />
      </Section><Divider /></>}</Show>
      <Section title="Heap detail"><KeyValues fields={heapDetail()} empty="Waiting for runtime samples" /></Section>
    </Pane>}>
    <Section title="Heap usage" detail={`${bytes(currentMemory("memory used size"))} used · ${bytes(currentMemory("malloc size"))} allocated`}>
      <InteractiveCanvas label="Heap allocation categories" height={128}
        project={(width, height) => { ws.runtime(); return core.projectJsMemoryChart(width, height); }} />
      <Legend entries={heap} />
    </Section>
    <Divider />
    <Bar><Tabs items={tabs} selected={tab()} select={setTab} /></Bar>
    <Divider />
    <Show when={tab() === "flamegraph"} fallback={
      <Show when={tab() === "console"} fallback={
        <Show when={tab() === "resources"} fallback={<Pane>
          <Section title="GC history">
            <Show when={(ws.runtime(), core.queryGcHistory().length)} fallback={<Label subtle>No GC measurements retained</Label>}>
              <For each={core.queryGcHistory()}>{row => <Tool label={`${row.name} · ${row.detail}`} onClick={() => {
                setGc(core.gcDetail(Number(row.id))); ws.showInspector(true);
              }} />}</For>
            </Show>
          </Section>
        </Pane>}>
          <Column style={{ flex: 1, minHeight: 0 }}>
            <View style={{ height: 145 }}><Pane>
              <Section title="Async counters"><KeyValues fields={(ws.resource(), core.jsResourceDetail())} /></Section>
            </Pane></View>
            <Divider />
            <Bar><Label weight={600}>Active timers</Label></Bar>
            <SliceTable query={(offset, limit) => { ws.resource(); return core.queryTimers(offset, limit); }}
              render={row => <TableRow row={row()} />} />
          </Column>
        </Show>
      }><ConsoleDrawer full /></Show>
    }>
      <Bar>
        <Tabs items={modes} selected={mode()} select={setMode} />
        <Search value={query()} change={setQuery} placeholder="Find a function" controller={ws.bindSearch} width={160} />
        <Show when={frame()}><Tool label={`Frame #${frame()} ×`} onClick={() => { setFrame(undefined); setZoom(0); }} active /></Show>
        <Spacer />
        <Show when={zoom() !== 0}><Tool label="Reset zoom" onClick={() => setZoom(0)} /></Show>
      </Bar>
      <Show when={mode() === "flame"} fallback={<ProfileTable mode={mode()} search={query()} frame={frame() ?? null} selected={hit()?.name} pick={row => {
        setHit({ id: row.id, name: row.name, detail: row.detail, fields: [], path: [] });
        ws.showInspector(true);
      }} />}>
        <Pane>
          <InteractiveCanvas label="JavaScript CPU flamegraph" height={320} project={plot} onSize={setSize} tip={tip()}
            onHover={point => { if (plot()?.rectBuckets.length) inspect(point, false); }} onLeave={() => { picked++; setTip(""); }}
            onClick={point => inspect(point, true)} onReset={() => setZoom(0)} />
          <Show when={!plot()?.rectBuckets.length}>
            <Empty title="No samples in this window" hint="Start sampling and interact with the target, or select a frame captured with detailed JS sampling." />
          </Show>
        </Pane>
      </Show>
      <View style={{ height: 1, backgroundColor: c().border }} />
    </Show>
  </PageLayout>;
}
