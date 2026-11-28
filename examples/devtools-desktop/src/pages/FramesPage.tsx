import { For, Show, createMemo, createSignal, createTrackedEffect, untrack } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { InteractiveCanvas } from "../charts/InteractiveCanvas";
import { Legend } from "../charts/Tracks";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Section, Spacer, Tabs, Tool, ms } from "../components/ui";
import { SliceTable, TableRow } from "../components/SliceTable";
import { PageLayout } from "../shell/PageLayout";
import { formatCadence, useWorkspace } from "../state/workspace";

const inspectorTabs = [{ id: "breakdown", label: "Breakdown" }, { id: "repaints", label: "Repainted views" }, { id: "js", label: "JS & reactive" }];
const pageTabs = [{ id: "timeline", label: "Timeline" }, { id: "capture", label: "Capture summary" }];
const frameLegend: readonly [string, number][] = [
  ["Script", 0], ["Layout", 1], ["Record", 2], ["Other CPU", 3], ["Raster", 4], ["GPU", 5], ["Compositor", 6], ["Over budget / missed present", 7],
];
const paintPaths = ["", "fast_layer_patch", "picture_patch", "full_picture_rebuild", "canvas_redraw"];

export function FramesPage() {
  const ws = useWorkspace();
  const [focus, select] = createSignal<string>();
  const [range, setRange] = createSignal<[string, string]>();
  const [pitch, setPitch] = createSignal(6);
  const [skip, setSkip] = createSignal(0);
  const [timeline, setTimeline] = createSignal<[number, number]>([0, 0]);
  const [tip, setTip] = createSignal("");
  const [span, setSpan] = createSignal<core.Hit>();
  const [mode, setMode] = createSignal("timeline");
  const [repaintPath, setRepaintPath] = createSignal("");
  const [repaintSort, setRepaintSort] = createSignal("");
  const [repaintMessage, setRepaintMessage] = createSignal("");
  const capture = createMemo(() => ws.tick()?.latestCapture ?? undefined);
  const count = createMemo(() => ws.tick()?.frameCount ?? 0);
  const frame = createMemo(() => focus() ?? ws.tick()?.latestFrame ?? undefined);
  const frameRevision = createMemo(() => {
    ws.frame(); ws.generation();
    const id = frame();
    return id ? core.frameRevision(id) : undefined;
  });

  function choose(id: string | null | undefined) {
    if (!id) return;
    select(id);
    setRange(undefined);
    setSpan(undefined);
    setTimeline([0, 0]);
    ws.showInspector(true);
  }
  createTrackedEffect(() => {
    const intent = ws.intent();
    if (intent.kind === "frame" && intent.frameId) choose(intent.frameId);
  });
  let previousStep = ws.frameStep();
  createTrackedEffect(() => {
    const delta = ws.frameStep() - previousStep;
    previousStep = ws.frameStep();
    if (delta) choose(core.stepFrame(untrack(frame) ?? null, delta));
  });
  let generation = ws.generation();
  createTrackedEffect(() => {
    if (generation === ws.generation()) return;
    generation = ws.generation();
    select(undefined);
    setRange(undefined);
    setSpan(undefined);
    setSkip(0);
    setTimeline([0, 0]);
  });
  createTrackedEffect(() => {
    const id = frame();
    const latest = capture();
    if (id && latest && ws.live() && core.captureContains(id)) void ws.run(() => core.loadFrameSpans(latest, id));
  });
  createTrackedEffect(() => {
    const frameId = frame();
    if (ws.frameTab() !== "repaints" || !frameId || !ws.live()) return;
    let active = true;
    setRepaintMessage("");
    void core.loadFrameRepaints(frameId).catch(() => {
      if (active) setRepaintMessage("No retained repaints for this frame. Select a newer frame while tracking is active.");
    });
    return () => { active = false; };
  });
  createTrackedEffect(() => { if (capture()) setMode("capture"); });
  const detail = createMemo(() => {
    frameRevision();
    const id = frame();
    return id ? core.getFrameDetail(id) : [];
  });
  const context = createMemo(() => {
    frameRevision(); ws.resource();
    const id = frame();
    return id ? core.frameContext(id) : [];
  });
  const compare = createMemo(() => {
    ws.frame();
    const ids = range();
    return ids ? core.compareFrameRange(...ids) : undefined;
  });
  const omittedSpans = createMemo(() => {
    frameRevision();
    const id = frame();
    return id ? core.omittedSpans(id) : 0;
  });
  const viewport = (width: number) => {
    const columns = Math.max(1, Math.min(Math.floor(width / pitch()), count() || 1));
    return { columns, start: Math.max(0, count() - columns - skip()) };
  };
  function hit(x: number, w: number) {
    const view = viewport(w);
    return core.hitTestFrameStrip(x, w, view.start, view.columns);
  }
  let origin: string | undefined;
  let ranged = false;
  let suppressClick = false;
  const inspector = <Column style={{ flex: 1, minHeight: 0 }}>
    <Bar><Tabs items={inspectorTabs} selected={ws.frameTab()} select={ws.setFrameTab} /></Bar>
    <Divider />
    <Show when={compare()} fallback={
      <Show when={frame()} fallback={<Empty title="No frames retained" hint="Connect to a target and interact with its window to generate frames." />}>
        {id => <>
          <Show when={ws.frameTab() === "repaints"} fallback={<Pane>
            <Show when={ws.frameTab() === "js"} fallback={<>
              <Show when={span()}>{bar => <><Section title={bar().name} detail={bar().detail}>
                <KeyValues fields={bar().fields} />
              </Section><Divider /></>}</Show>
              <Section title={`Frame #${id()}`}>
                <For each={context().filter(row => row.kind === "block")} keyed={row => row.id}>{row =>
                  <Tool label={`Blocked · ${row().name}`} icon="chevron" onClick={() =>
                    ws.nav.go({ kind: "view", nodeId: row().id, highlight: true, fromFrameId: id() })} />
                }</For>
                <KeyValues fields={detail()} empty="Frame is outside the retained window" />
              </Section>
            </>}>
              <Section title={`Frame #${id()} · JS & reactive`}>
                <Tool label="Open in flamegraph" icon="chevron" onClick={() => ws.nav.go({ kind: "js", subTab: "flamegraph", frameId: id() })} />
                <KeyValues fields={detail().filter(field => /updates|sets|nodes created|ffi |style resolves|callback ms|canvas |script ms/.test(field.name))} />
                <For each={context().filter(row => row.kind === "stack")} keyed={row => row.id}>{row => <Label mono size={11} lines={5}>{row().detail}</Label>}</For>
              </Section>
            </Show>
          </Pane>}>
            <Section title={`Repaints in #${id()}`}>
              <KeyValues fields={(ws.repaint(), core.frameRepaintSummary(id()))} empty={repaintMessage() || "Waiting for repaint data"} />
              <Row style={{ gap: 4, flexWrap: "wrap" }}>
                <Tool label={repaintPath() || "All paths"} onClick={() => setRepaintPath(paintPaths[(paintPaths.indexOf(repaintPath()) + 1) % paintPaths.length])} />
                <Tool label={repaintSort() ? "By commands" : "Frame order"} onClick={() => setRepaintSort(repaintSort() ? "" : "commands")} />
              </Row>
            </Section>
            <SliceTable query={(offset, limit) => {
              ws.repaint(); return core.queryFrameRepaints(id(), repaintPath(), repaintSort(), offset, limit);
            }} render={row => <TableRow row={row()} onClick={() => ws.nav.go({ kind: "view", nodeId: row().id,
              highlight: true, fromFrameId: id() })} />} />
          </Show>
        </>}
      </Show>
    }>{range => <Pane>
      <Section title="Range comparison" detail={`${range().selected.count} selected`}>
        <KeyValues fields={([
          ["Hitch (ms/s)", (stats) => stats.hitchMsPerSec.toFixed(1)],
          ["Dropped", (stats) => String(stats.droppedFrames)],
          ["On-time", (stats) => `${stats.onTimePct.toFixed(1)}%`],
          ["Cadence", formatCadence],
          ["P95 bottleneck", (stats) => ms(stats.p95Ms)],
          ["P95 E2E", (stats) => ms(stats.p95E2eMs)],
          ["UI / Raster / GPU over", (stats) => `${stats.uiJankCount} / ${stats.rasterJankCount} / ${stats.gpuJankCount}`],
        ] satisfies [string, (stats: core.FrameStats) => string][]).map(([name, format]) => ({
          name, value: `${format(range().selected)} / ${format(range().all)}`, depth: 0,
        }))} />
        <Label subtle>Selected / retained frames</Label>
        <Tool label="Clear selection" onClick={() => setRange(undefined)} />
      </Section>
    </Pane>}</Show>
  </Column>;
  return <PageLayout title="Frames" inspector={inspector} inspectorTitle="Frame inspector"
    actions={<>
      <Tool label="Worst frame" icon="warning" disabled={!count()} onClick={() => choose(core.worstFrame())} />
      <Tool label={ws.recordDetail() === "stages" ? "Stages" : "Detailed + JS"} disabled={!!ws.capture()}
        onClick={() => ws.setRecordDetail(ws.recordDetail() === "stages" ? "detailed" : "stages")} />
    </>}>
    <Section title="Frame strip" detail={`${count()} retained`}>
      <InteractiveCanvas label="Frame strip" height={110} interactive tip={tip()}
        project={(width, height) => {
          ws.frame();
          const view = viewport(width);
          return core.projectFrameStrip(width, height, view.start, view.columns, frame(), range()?.[0], range()?.[1]);
        }}
        onHover={point => setTip(hit(point.x, point.width)?.detail ?? "")}
        onPress={point => { origin = hit(point.x, point.width)?.id; ranged = false; suppressClick = false; }}
        onClick={point => { if (!suppressClick) choose(hit(point.x, point.width)?.id); }}
        onDrag={point => {
          const target = hit(Math.max(0, Math.min(point.width - 0.001, point.x)), point.width)?.id;
          if (origin && target && (ranged || origin !== target)) {
            ranged = true;
            setRange([origin, target]);
            ws.showInspector(true);
          }
        }}
        onDragEnd={() => { suppressClick = ranged; }}
        onWheel={(dx, dy, size) => {
          const delta = Math.abs(dx) >= Math.abs(dy) ? dx : dy;
          if (delta) setSkip(value => Math.max(0, Math.min(count() - viewport(size.width).columns, value + Math.round(delta / pitch()))));
        }}
        onZoom={scale => setPitch(value => Math.min(18, Math.max(2, value * scale)))}
        onKey={key => {
          if (key === "Left" || key === "Right") choose(core.stepFrame(frame() ?? null, key === "Left" ? -1 : 1));
          if (key === "Escape") setRange(undefined);
          if (key === " " || key === "Space") void ws.togglePause();
        }} />
      <Legend entries={frameLegend} />
    </Section>
    <Divider />
    <Bar>
      <Tabs items={pageTabs} selected={mode()} select={setMode} />
      <Spacer />
      <Tool label="Follow latest" disabled={!focus() && skip() === 0} onClick={() => { select(undefined); setSkip(0); setRange(undefined); }} />
      <Tool label="−" onClick={() => setPitch(value => Math.max(2, value - 1))} />
      <Tool label="+" onClick={() => setPitch(value => Math.min(18, value + 1))} />
    </Bar>
    <Divider />
    <Show when={mode() === "capture"} fallback={<Pane>
      <Show when={frame()} fallback={<Empty title="Frame timeline" hint="The platform, UI, raster, I/O and GPU lanes appear when a frame is available." />}>
        {id => <Section title={`Frame #${id()}`} detail={omittedSpans() ? `${omittedSpans()} spans omitted` : "GPU clock is relative"}>
          <InteractiveCanvas label="Five-lane frame timeline" height={310} interactive
            project={(w, h) => { frameRevision(); return core.projectFrameGantt(id(), w, h, ...timeline()); }}
            onClick={point => {
              setSpan(core.hitTestFrameGantt(id(), point.x, point.y, point.width, point.height, ...timeline()) ?? undefined);
              ws.setFrameTab("breakdown");
              ws.showInspector(true);
            }}
            onWheel={(dx, dy) => {
              const [start, end] = timeline();
              if (end <= start) return;
              const shift = (dx || dy) * (end - start) / 500;
              const next = Math.max(0, start + shift);
              setTimeline([next, next + end - start]);
            }}
            onZoom={scale => {
              const [start, end] = timeline();
              const initial = Number(detail().find(field => field.name === "e2e ms")?.value) || 16.67;
              setTimeline([start, start + Math.max(0.1, Math.min(10_000, ((end - start) || initial) / scale))]);
            }}
            onReset={() => setTimeline([0, 0])} />
          <Tool label="Reset time scale" onClick={() => setTimeline([0, 0])} />
        </Section>}
      </Show>
    </Pane>}>
      <Show when={capture()} fallback={<Empty title="No completed capture" hint="Record a trace to inspect recorded spans and export a Perfetto file." />}>
        {id => <Column style={{ flex: 1, minHeight: 0 }}>
          <Section title={`Capture #${id()}`}>
            <Row style={{ gap: 6 }}>
              <Tool label="Export .pftrace" icon="export" onClick={() => void ws.saveTrace(id())} />
              <Tool label="Open Perfetto" icon="open" onClick={() => void ws.run(() => core.openInPerfetto(id()))} />
            </Row>
            <KeyValues fields={core.captureDetail()} />
          </Section>
          <SliceTable query={core.captureSummary} render={row => <TableRow row={row()} onClick={() => { choose(row().id); setMode("timeline"); }} />} />
        </Column>}
      </Show>
    </Show>
  </PageLayout>;
}
