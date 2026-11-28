import { For, Show, createMemo, createSignal, createTrackedEffect } from "solid-js";
import { Column, Row } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Section, Tabs, Tool, bytes } from "../components/ui";
import { TableRow, SliceTable } from "../components/SliceTable";
import { InteractiveCanvas } from "../charts/InteractiveCanvas";
import { Tracks, type Track } from "../charts/Tracks";
import { PageLayout } from "../shell/PageLayout";
import { useWorkspace } from "../state/workspace";

const tabs = [{ id: "trends", label: "Runtime resources" }, { id: "bundle", label: "Bundle analyzer" }, { id: "archive", label: "Session & artifacts" }];
const tracks: readonly Track[] = [
  { label: "Resident memory", legend: [["Atlas", 48], ["Offscreen peak", 51], ["Images", 49], ["JS heap", 52]], unit: "MiB" },
  { label: "Scene objects", legend: [["Atlas entries", 48], ["Textures", 51], ["Pictures", 49], ["Layers", 52]], unit: "" },
  { label: "Atlas pressure", legend: [["Hit rate (%)", 49], ["Evictions", 52]], unit: "" },
];

export function ResourcesPage() {
  const ws = useWorkspace();
  const [tab, setTab] = createSignal("trends");
  const [path, setPath] = createSignal<string[]>([]);
  const [metric, setMetric] = createSignal("bytecode");
  const [hit, setHit] = createSignal<core.Hit>();
  const [tooltip, setTooltip] = createSignal("");
  const [assets, setAssets] = createSignal(false);
  const bundle = createMemo(() => ws.tick()?.bundleLoaded ?? false);
  const artifact = ws.artifact;
  createTrackedEffect(() => {
    const intent = ws.intent();
    if (intent.kind === "resources") setTab(intent.subTab ?? "trends");
  });
  createTrackedEffect(() => {
    artifact()?.name;
    setPath([]);
    setHit(undefined);
  });
  return <PageLayout title="Resources & Bundle" actions={<Tool label="Open artifact" icon="open" onClick={() => void ws.openArtifact()} />}
    inspectorTitle={tab() === "bundle" ? "Bundle inspector" : "Resources inspector"}
    inspector={<Show when={tab() === "bundle" && bundle()} fallback={<Pane>
      <Section title="Latest frame"><KeyValues fields={(ws.frame(), core.resourceDetail())} empty="Connect to capture resource metrics" /></Section>
      <Show when={artifact()}>{file => <><Divider /><Section title="Loaded artifact" detail={bytes(file().bytes)}>
        <Label lines={3}>{file().name}</Label><Label subtle>{file().kind}</Label>
        <Show when={file().packets != null}><Label mono subtle>{file().packets} packets</Label></Show>
      </Section></>}</Show>
    </Pane>}>
      <Column style={{ flex: 1, minHeight: 0 }}>
        <Show when={hit()}>{item => <><Section title={item().name} detail={item().detail}>
          <KeyValues fields={item().fields} />
        </Section><Divider /></>}</Show>
        <Bar><Tabs items={[{ id: "modules", label: "Top modules" }, { id: "assets", label: "External assets" }]}
          selected={assets() ? "assets" : "modules"} select={id => setAssets(id === "assets")} /></Bar>
        <Divider />
        <SliceTable reset={String(assets())} query={(offset, limit) => {
          ws.resource(); return core.queryBundleRows(assets(), offset, limit);
        }} render={row => <TableRow row={row()} />}
          empty={assets() ? "No external assets in the report" : "No modules in the report"} />
      </Column>
    </Show>}>
    <Bar><Tabs items={tabs} selected={tab()} select={setTab} /></Bar>
    <Divider />
    <Show when={tab() === "trends"} fallback={
      <Show when={tab() === "bundle"} fallback={<Pane>
        <Section title="Session recording">
          <Label subtle lines={3}>Export the retained telemetry for offline inspection. ZIP can include a staged Perfetto capture.</Label>
          <Row style={{ gap: 4, flexWrap: "wrap" }}>
            <Tool label="Export session ZIP" icon="export" disabled={!ws.tick()?.session} onClick={() => void ws.saveSession("zip")} />
            <Tool label="Export .impasto-session" disabled={!ws.tick()?.session} onClick={() => void ws.saveSession("gzip")} />
            <Tool label="Open recording" icon="open" onClick={() => void ws.openArtifact()} />
          </Row>
        </Section>
        <Divider />
        <Section title="Artifacts">
          <Show when={artifact()} fallback={<Label subtle>Open a bundle report, symbols, or a Perfetto trace from disk</Label>}>
            {file => <>
              <Label>{file().name}</Label>
              <Label subtle>{file().kind} · {bytes(file().bytes)}</Label>
              <Show when={file().kind === "Perfetto trace"}>
                <Tool label="Open in Perfetto" icon="open" onClick={() => void ws.run(() => core.openInPerfetto("0"))} />
              </Show>
            </>}
          </Show>
          <Tool label="Open artifact" icon="open" onClick={() => void ws.openArtifact()} />
        </Section>
      </Pane>}>
        <Show when={bundle()} fallback={<Empty title="Open a bundle size report"
          hint="Generate bundle-size.json with impasto build bundle --analyze-size."
          action={<Tool label="Open bundle report" icon="open" onClick={() => void ws.openArtifact()} />} />}>
          <Pane>
            <Section title="Bundle size" detail={metric() === "bytecode" ? "Apportioned bytecode" : "Rendered JavaScript"}>
              <KeyValues fields={(ws.resource(), core.bundleSummary())} />
              <Tabs items={[{ id: "bytecode", label: "Apportioned bytecode" }, { id: "rendered", label: "Rendered JS" }]}
                selected={metric()} select={value => { setMetric(value); setHit(undefined); }} />
              <Row style={{ gap: 4, flexWrap: "wrap" }}>
                <Tool label="All chunks" active={path().length === 0} onClick={() => { setPath([]); setHit(undefined); }} />
                <For each={path()}>{(name, index) => <Tool label={name} icon="chevron" active={index() === path().length - 1}
                  onClick={() => { setPath(path().slice(0, index() + 1)); setHit(undefined); }} />}</For>
              </Row>
              <InteractiveCanvas label="Squarified bundle treemap" height={380} interactive tip={tooltip()}
                project={(width, height) => { ws.resource(); return core.projectBundleTreemap(width, height, path(), metric()); }}
                onHover={point => {
                  const found = core.hitTestBundleTreemap(point.x, point.y, point.width, point.height, path(), metric());
                  setTooltip(found ? `${found.id} · ${found.detail}` : "");
                }}
                onClick={point => {
                  const found = core.hitTestBundleTreemap(point.x, point.y, point.width, point.height, path(), metric());
                  if (!found) return;
                  setHit(found);
                  ws.showInspector(true);
                  if (found.path.length) setPath(found.path);
                }} onReset={() => { setPath([]); setHit(undefined); }} />
            </Section>
          </Pane>
        </Show>
      </Show>
    }>
      <Pane><Section title="Resource trends">
        <Tracks tracks={tracks} height={100}
          project={(width, height) => { ws.frame(); ws.runtime(); return core.projectResourceTrends(width, height); }} />
      </Section></Pane>
    </Show>
  </PageLayout>;
}
