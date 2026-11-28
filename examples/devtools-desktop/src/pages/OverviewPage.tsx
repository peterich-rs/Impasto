import { For, Show, createMemo, createSignal } from "solid-js";
import { Column, RepaintBoundary, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { PageLayout } from "../shell/PageLayout";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Section, Tabs, Tool, bytes, ms } from "../components/ui";
import { Tracks, type Track } from "../charts/Tracks";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";
import { Icon } from "../components/Icons";

const tracks: readonly Track[] = [
  { label: "Frame bottleneck", legend: [["Bottleneck", 84], ["Budget", 81]], unit: "ms" },
  { label: "Pipeline", legend: [["UI", 0], ["Raster", 4], ["GPU", 5]], unit: "ms" },
  { label: "JavaScript heap", legend: [["Used", 48]], unit: "MiB" },
  { label: "Network", legend: [["Active requests", 22], ["Latency (s)", 23]], unit: "" },
];
const windows = ["10", "30", "60"].map(id => ({ id, label: `${id}s` }));

export function OverviewPage() {
  const ws = useWorkspace();
  const c = useTokens();
  const [window, setWindow] = createSignal("30");
  const [selected, select] = createSignal("");
  const audits = createMemo(() => { ws.audit(); return core.getLiveAudits(); });
  const finding = () => audits().find(finding => finding.id === selected());
  const evidence = createMemo(() => {
    const issue = finding();
    if (!issue) return [];
    if (issue.page === "frame") ws.frame();
    else if (issue.page === "view") ws.repaint();
    else if (issue.page === "network") ws.networkRecords();
    else if (issue.page === "js") ws.runtime();
    return core.auditEvidence(issue.id);
  });
  const session = ws.session;
  const stats = () => ws.tick()?.stats;
  const hitchNote = createMemo(() => {
    const sample = stats();
    if (!sample?.count) return ws.cadenceLabel();
    if (sample.droppedFrames) {
      const stages = [
        [sample.uiJankCount, "UI"], [sample.rasterJankCount, "Raster"], [sample.gpuJankCount, "GPU"],
      ] as const;
      const attribution = stages.filter(([count]) => count > 0).map(([count, stage]) => `${count} ${stage}`).join(", ");
      return `${sample.droppedFrames} dropped${attribution ? ` (${attribution})` : ""} · ${ws.cadenceLabel()}`;
    }
    const hz = session()?.refreshRate ?? (sample.budgetMs > 0 ? 1000 / sample.budgetMs : 0);
    return `${sample.onTimePct.toFixed(1).replace(/\.0$/, "")}% on-time · ${ws.cadenceLabel()}${hz ? ` · ${Math.round(hz)} Hz` : ""}`;
  });
  const frameNote = createMemo(() => {
    const sample = stats();
    if (!sample?.count) return "—";
    const budget = sample.budgetMs > 0 ? `${ms(sample.budgetMs)} budget · ` : "";
    const over = sample.jankCount ? ` · ${sample.jankCount} over` : "";
    return `${budget}E2E ${ms(sample.p95E2eMs)}${over}`;
  });
  const metrics = [
    { title: "Hitch", value: ws.hitchLabel, note: hitchNote,
      tone: () => { const tone = ws.hitchTone(); return tone ? c()[tone] : undefined; } },
    { title: "Frame P95", value: createMemo(() => stats()?.count ? ms(stats()?.p95Ms) : "—"),
      note: frameNote },
    { title: "Compositor only", value: createMemo(() => stats()?.count ? `${stats()!.compositorPct.toFixed(0)}%` : "—"),
      note: () => "Fast-path frames" },
    { title: "JS heap", value: createMemo(() => ws.tick()?.jsHeapBytes ? bytes(ws.tick()!.jsHeapBytes) : "—"),
      note: () => "Memory used" },
    { title: "Network", value: createMemo(() => String(ws.tick()?.networkCount ?? 0)),
      note: createMemo(() => `${ws.tick()?.activeNetCount ?? 0} active`) },
    { title: "Issues", value: createMemo(() => String((ws.tick()?.errorCount ?? 0) + audits().length)),
      note: createMemo(() => `${audits().length} active audits`) },
  ];
  const frameEnd = createMemo(() => { ws.frame(); ws.generation(); return core.overviewWindowEnd(0); });
  const heapEnd = createMemo(() => { ws.runtime(); ws.generation(); return core.overviewWindowEnd(2); });
  const networkEnd = createMemo(() => { ws.networkRecords(); ws.generation(); return core.overviewWindowEnd(3); });
  // Every track shares the same time axis. Move it in one-second steps.
  const end = createMemo(() => Math.ceil(Math.max(frameEnd(), heapEnd(), networkEnd()) / 1000) * 1000);
  function inspect(issue: core.Finding) {
    if (issue.page === "frame") ws.nav.go({ kind: "frame", frameId: issue.selection, tab: "breakdown" });
    if (issue.page === "view") ws.nav.go({ kind: "view", nodeId: issue.selection || undefined,
      mode: issue.tab === "layout" ? "layout" : "tree", highlight: !!issue.selection });
    if (issue.page === "network") ws.nav.go({ kind: "network", token: issue.selection || undefined,
      tab: "timing", status: issue.tab === "error" ? "error" : undefined });
    if (issue.page === "js") ws.nav.go({ kind: "js", subTab: "resources" });
  }
  const environment = createMemo(() => {
    const target = session();
    if (!target) return [];
    return [
      ["Application", target.appName], ["Platform", target.platform], ["Process", String(target.pid)],
      ["OS", target.os], ["Framework", target.frameworkVersion],
      ["Viewport", target.width && target.height ? `${target.width} × ${target.height} @ ${target.dpr}x` : "—"],
      ["Refresh", target.refreshRate ? `${Math.round(target.refreshRate)} Hz` : "—"],
      ["Thread topology", target.topology ?? "—"],
    ].map(([name, value]) => ({ name, value, depth: 0 }));
  });
  return <PageLayout title="Overview" actions={<Tabs items={windows} selected={window()} select={setWindow} />}
    inspectorTitle={finding() ? "Audit evidence" : "Session"}
    inspector={<Pane>
      <Show when={finding()} fallback={<>
        <Section title="Target environment"><KeyValues fields={environment()} empty="Select a target to inspect its environment" /></Section>
        <Show when={ws.capabilities().length > 0}>
          <Divider />
          <Section title="Agent capabilities">
            <For each={ws.capabilities()}>{value => <Label size={11} mono subtle>{value.replace(/_/g, " ")}</Label>}</For>
          </Section>
        </Show>
      </>}>{issue => <Section title={issue().title}>
        <Label mono tone={c().warn}>{issue().metric}</Label>
        <Label lines={5} subtle>{issue().summary}</Label>
        <For each={evidence()} keyed={row => row.id}>{row =>
          <Column style={{ gap: 3, paddingTop: 5, paddingBottom: 5 }}>
            <Label lines={2}>{row().name}</Label>
            <Label mono subtle>{row().detail}</Label>
          </Column>
        }</For>
        <Tool label="Inspect evidence" icon="chevron" onClick={() => inspect(issue())} />
      </Section>}</Show>
    </Pane>}>
    <Pane>
      <Show when={session()} fallback={<Empty title="Waiting for a target"
        hint="Launch an Impasto app with DevTools enabled, or open a recording to inspect a previous session."
        action={<Tool label="Open recording" icon="open" onClick={() => { void ws.openArtifact(); }} />} />}>
        <RepaintBoundary>
          <Row style={{ backgroundColor: c().panel, minHeight: 76, flexWrap: "wrap" }}>
            <For each={metrics}>{(metric, i) => <>
              <Show when={i() > 0}><Divider vertical /></Show>
              <Column style={{ flex: 1, minWidth: 85, gap: 4, padding: 11 }}>
                <Label size={11} subtle>{metric.title}</Label>
                <Label size={20} weight={600} mono tone={metric.tone?.()}>{metric.value()}</Label>
                <Label size={10} lines={3} subtle tone={metric.title === "Frame P95" && (stats()?.jankCount ?? 0) > 0 ? c().warn : undefined}>{metric.note()}</Label>
              </Column>
            </>}</For>
          </Row>
        </RepaintBoundary>
        <Divider />
      </Show>
      <Section title="Activity" detail={`Last ${window()} seconds`}>
        <Tracks tracks={tracks} projectTrack={(index, w, h) => {
          ws.generation();
          if (index < 2) ws.frame();
          else if (index === 2) ws.runtime();
          else ws.networkRecords();
          return core.projectOverviewTrack(index, w, h, Number(window()), end()) ?? undefined;
        }} onClick={(time, index) => {
          if (index <= 1) {
            const frameId = core.frameNearTime(time);
            if (frameId) ws.nav.go({ kind: "frame", frameId });
          } else if (index === 2) ws.nav.go({ kind: "js", subTab: "memory" });
          else ws.nav.go({ kind: "network" });
        }} />
      </Section>
      <Divider />
      <Section title="Live audits" detail={String(audits().length)}>
        <Show when={audits().length} fallback={<Label subtle>{session() ? "No issues in the retained telemetry" : "Audits appear as telemetry arrives"}</Label>}>
          <For each={audits()} keyed={issue => issue.id}>{issue => <View style={{ backgroundColor: selected() === issue().id ? c().selected : c().panel, borderRadius: 4 }}>
            <Bar><Icon name="warning" color={issue().severity === "critical" ? c().danger : issue().severity === "warning" ? c().warn : c().info} />
              <Column style={{ flex: 1, minWidth: 0 }}>
                <Label weight={600}>{issue().title}</Label>
                <Label subtle>{issue().summary}</Label>
              </Column>
              <Label mono size={11} subtle>{issue().metric}</Label>
              <Tool label="Evidence" onClick={() => { select(issue().id); ws.showInspector(true); }} />
              <Tool label="Inspect" icon="chevron" onClick={() => inspect(issue())} />
            </Bar>
          </View>}</For>
        </Show>
      </Section>
    </Pane>
  </PageLayout>;
}
