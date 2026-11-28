import { For, Show, createMemo, createSignal } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { PageLayout } from "../shell/PageLayout";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Section, Tabs, Tool, bytes, ms } from "../components/ui";
import { Tracks, type Track } from "../charts/Tracks";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";
import { Icon } from "../components/Icons";

const tracks: readonly Track[] = [
  { label: "Frame total", legend: [["Frame", 84], ["Budget", 81]], unit: "ms" },
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
  const session = createMemo(() => ws.tick()?.session);
  const stats = () => ws.tick()?.stats;
  const numbers = () => [
    ["FPS", stats()?.fps ? stats()!.fps.toFixed(0) : "—", session()?.refreshRate ? `Target ${Math.round(session()?.refreshRate ?? 0)} Hz` : "Presented cadence"],
    ["Frame P95", stats()?.count ? ms(stats()?.p95Ms) : "—", `${stats()?.jankCount ?? 0} over budget`],
    ["Compositor only", stats()?.count ? `${stats()!.compositorPct.toFixed(0)}%` : "—", "Fast-path frames"],
    ["JS heap", ws.tick()?.jsHeapBytes ? bytes(ws.tick()!.jsHeapBytes) : "—", "Memory used"],
    ["Network", String(ws.tick()?.networkCount ?? 0), `${ws.tick()?.activeNetCount ?? 0} active`],
    ["Issues", String((ws.tick()?.errorCount ?? 0) + audits().length), `${audits().length} active audits`],
  ];
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
        <Show when={(ws.tick()?.capabilities.length ?? 0) > 0}>
          <Divider />
          <Section title="Agent capabilities">
            <For each={ws.tick()?.capabilities}>{value => <Label size={11} mono subtle>{value.replace(/_/g, " ")}</Label>}</For>
          </Section>
        </Show>
      </>}>{issue => <Section title={issue().title}>
        <Label mono tone={c().warn}>{issue().metric}</Label>
        <Label lines={5} subtle>{issue().summary}</Label>
        <For each={(ws.audit(), core.auditEvidence(issue().id))}>{row =>
          <Column style={{ gap: 3, paddingTop: 5, paddingBottom: 5 }}>
            <Label lines={2}>{row.name}</Label>
            <Label mono subtle>{row.detail}</Label>
          </Column>
        }</For>
        <Tool label="Inspect evidence" icon="chevron" onClick={() => inspect(issue())} />
      </Section>}</Show>
    </Pane>}>
    <Pane>
      <Show when={session()} fallback={<Empty title="Waiting for a target"
        hint="Launch an Impasto app with DevTools enabled, or open a recording to inspect a previous session."
        action={<Tool label="Open recording" icon="open" onClick={() => { void ws.openArtifact(); }} />} />}>
        <Row style={{ backgroundColor: c().panel, minHeight: 76, flexWrap: "wrap" }}>
          <For each={numbers()}>{([title, value, note], i) => <>
            <Show when={i() > 0}><Divider vertical /></Show>
            <Column style={{ flex: 1, minWidth: 85, gap: 4, padding: 11 }}>
              <Label size={11} subtle>{title}</Label>
              <Label size={20} weight={600} mono>{value}</Label>
              <Label size={10} subtle>{note}</Label>
            </Column>
          </>}</For>
        </Row>
        <Divider />
      </Show>
      <Section title="Activity" detail={`Last ${window()} seconds`}>
        <Tracks tracks={tracks} project={(w, h) => {
          ws.frame(); ws.runtime(); ws.network();
          return core.projectOverviewTracks(w, h, Number(window()));
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
          <For each={audits()}>{issue => <View style={{ backgroundColor: selected() === issue.id ? c().selected : c().panel, borderRadius: 4 }}>
            <Bar><Icon name="warning" color={issue.severity === "critical" ? c().danger : issue.severity === "warning" ? c().warn : c().info} />
              <Column style={{ flex: 1, minWidth: 0 }}>
                <Label weight={600}>{issue.title}</Label>
                <Label subtle>{issue.summary}</Label>
              </Column>
              <Label mono size={11} subtle>{issue.metric}</Label>
              <Tool label="Evidence" onClick={() => { select(issue.id); ws.showInspector(true); }} />
              <Tool label="Inspect" icon="chevron" onClick={() => inspect(issue)} />
            </Bar>
          </View>}</For>
        </Show>
      </Section>
    </Pane>
  </PageLayout>;
}
