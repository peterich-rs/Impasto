import { For, Show, createMemo, createSignal, createTrackedEffect, onCleanup, untrack } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Search, Section, Spacer, Tabs, Tool } from "../components/ui";
import { SourceLink } from "../components/SourceLink";
import { SliceTable, TableRow } from "../components/SliceTable";
import { PageLayout } from "../shell/PageLayout";

import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";

const modes = [{ id: "tree", label: "Tree" }, { id: "leaderboard", label: "Repaint leaderboard" },
  { id: "layout", label: "Layout issues" }, { id: "layers", label: "Layer tree" }];
const paths = ["", "full_picture_rebuild", "picture_patch", "canvas_redraw", "fast_layer_patch"];

export function ViewsPage() {
  const ws = useWorkspace();
  const c = useTokens();
  const [mode, setMode] = createSignal("tree");
  const [layerMode, setLayerMode] = createSignal("layers");
  const [query, setQuery] = createSignal("");
  const [filter, setFilter] = createSignal("");
  const [expanded, setExpanded] = createSignal<string[]>([]);
  const [selected, select] = createSignal("");
  const [layer, selectLayer] = createSignal("");
  const [locate, setLocate] = createSignal<number>();
  const [layerFrame, setLayerFrame] = createSignal<string>();
  const overlay = ws.overlay;
  const deepInspect = createMemo(() => ws.tick()?.profile === "deep_inspect");
  let hover: ReturnType<typeof setTimeout> | undefined;

  function find(node: string) {
    if (!node) return;
    const ancestors = core.viewAncestors(node);
    const expandedIds = [...new Set([...expanded(), ...ancestors])];
    setExpanded(expandedIds);
    setLocate(core.locateView(node, expandedIds) ?? undefined);
  }
  function selectedNode(node: string, highlight = true) {
    select(node);
    ws.showInspector(true);
    if (highlight && ws.live()) void ws.updateOverlay({ highlightNode: node });
  }
  async function refresh(kind: string, inspect = true) {
    if (!ws.live()) return;
    if (kind === "tree") {
      await core.refreshViewSnapshot(2000);
      if (selected()) find(selected());
    }
    if (kind === "leaderboard") await core.refreshRepaintLeaderboard(100);
    if (kind === "layout") await core.refreshLayoutIssues(inspect);
    if (kind === "layers") await core.loadLayerTree(layerFrame() ?? null);
  }
  createTrackedEffect(() => {
    const intent = ws.intent();
    if (intent.kind !== "view") return;
    setMode(intent.mode ?? "tree");
    setLayerFrame(intent.fromFrameId);
    if (intent.nodeId) {
      setQuery("");
      setFilter("");
      select(intent.nodeId);
      untrack(() => find(intent.nodeId!));
    }
  });
  createTrackedEffect(() => {
    const kind = mode();
    const live = ws.live();
    const enabled = deepInspect();
    if (live && (kind !== "layers" || enabled)) untrack(() => { void ws.run(() => refresh(kind)); });
  });
  onCleanup(() => {
    if (hover !== undefined) clearTimeout(hover);
    if (ws.live() && ws.overlay().highlightNode) void ws.updateOverlay({ highlightNode: undefined });
  });
  const source = createMemo(() => {
    ws.repaint();
    return selected() ? core.nodeSource(selected()) : [];
  });
  const part = (name: string) => source().find(field => field.name === name)?.value;
  const detail = createMemo(() => {
    ws.repaint();
    return mode() === "layers"
      ? core.viewInspectionDetail(layerMode(), layer())
      : mode() !== "tree" ? core.viewInspectionDetail(mode(), selected())
      : selected() ? core.getViewNodeDetail(selected()) : [];
  });
  const dimension = (name: string) => Number(detail().find(field => field.name === name)?.value) || 0;
  const boxScale = () => Math.min(1, 210 / Math.max(1, dimension("w")), 50 / Math.max(1, dimension("h")));
  const repaintFrames = createMemo(() => { ws.repaint(); return selected() ? core.nodeFrames(selected()) : []; });
  return <PageLayout title="Views" inspectorTitle={mode() === "layers" ? "Layer inspector" : "View inspector"}
    actions={<>
      <Tool label="Pick" icon="pick" active={overlay().pickMode} disabled={!ws.live()} onClick={() => void ws.updateOverlay({ pickMode: !overlay().pickMode })} />
      <Tool label="Rainbow" active={overlay().repaintRainbow} disabled={!ws.live()} onClick={() => void ws.updateOverlay({ repaintRainbow: !overlay().repaintRainbow })} />
      <Tool label="Layout scan" active={overlay().layoutScan} disabled={!ws.live()} onClick={() => void ws.updateOverlay({ layoutScan: !overlay().layoutScan })} />
      <Tool label="Refresh inspection" icon="refresh" iconOnly disabled={!ws.live()} onClick={() => void ws.run(() => refresh(mode()))} />
    </>}
    inspector={<Pane>
      <Show when={mode() === "layers" ? layer() : selected()} fallback={<Empty title="Select a view" hint="Inspect its layout, repaint history and original source." />}>
        <Show when={mode() === "tree" && selected()}>
          <Section title="Scene box">
            <View style={{ height: 82, alignItems: "center", justifyContent: "center", backgroundColor: c().canvas }}>
              <View style={{ width: Math.max(30, dimension("w") * boxScale()), height: Math.max(18, dimension("h") * boxScale()),
                backgroundColor: c().selected, borderWidth: 1, borderColor: c().accent, alignItems: "center", justifyContent: "center" }}>
                <Label mono size={10}>{Math.round(dimension("w"))} × {Math.round(dimension("h"))}</Label>
              </View>
            </View>
            <KeyValues fields={detail().filter(field => ["x", "y", "w", "h", "focusable", "kind"].includes(field.name))} />
          </Section>
          <Divider />
        </Show>
        <Show when={part("path")}>
          <Section title={part("component") || "Source"}>
            <SourceLink text={`${part("path")}:${part("line")}:${part("column")}`}
              path={part("path")} line={Number(part("line"))} column={Number(part("column"))} />
            <Tool label={`Editor · ${ws.editor()}`} onClick={() => ws.setEditor(ws.editor() === "vscode" ? "cursor" : ws.editor() === "cursor" ? "zed" : "vscode")} />
          </Section>
          <Divider />
        </Show>
        <Section title="Details">
          <KeyValues fields={detail()} />
          <Show when={mode() === "layout" || mode() === "leaderboard"}>
            <Tool label="Locate in tree" icon="chevron" onClick={() => ws.nav.go({ kind: "view", nodeId: selected(), highlight: true })} />
          </Show>
        </Section>
        <Show when={repaintFrames().length > 0}>
          <Divider /><Section title="Repainted in frames"><Row style={{ gap: 4, flexWrap: "wrap" }}>
            <For each={repaintFrames()}>{id => <Tool label={`#${id}`} onClick={() => ws.nav.go({ kind: "frame", frameId: id, tab: "repaints" })} />}</For>
          </Row></Section>
        </Show>
      </Show>
    </Pane>}>
    <Bar><Tabs items={modes} selected={mode()} select={next => { setMode(next); setLocate(undefined); }} /></Bar>
    <Divider />
    <Show when={mode() === "tree"} fallback={
      <Show when={mode() === "layers" && ws.live() && ws.tick()?.profile !== "deep_inspect"} fallback={<>
        <Show when={mode() === "layers"}>
          <Bar><Tabs items={[{ id: "layers", label: "Layers" }, { id: "pictures", label: "Pictures" }]} selected={layerMode()} select={setLayerMode} />
            <Spacer /><Label mono subtle>{layerFrame() ? `Frame #${layerFrame()}` : "Latest retained tree"}</Label></Bar>
        </Show>
        <SliceTable reset={`${mode()}|${layerMode()}`} query={(offset, limit) => {
          ws.repaint(); return core.queryViewInspection(mode() === "layers" ? layerMode() : mode(), offset, limit);
        }} render={row => <TableRow row={row()} selected={(mode() === "layers" ? layer() : selected()) === row().id} onClick={() => {
          if (mode() === "layers") { selectLayer(row().id); ws.showInspector(true); }
          else if (mode() === "layout") {
            ws.nav.go({ kind: "view", mode: "tree", nodeId: row().id, highlight: true });
          } else selectedNode(row().id);
        }}><Show when={row().count}><Label mono size={11} tone={c().warn}>{mode() === "layout" ? `+${row().count.toFixed(1)} px` : row().count}</Label></Show></TableRow>} />
      </>}>
        <Empty title="Layer tree capture is off" hint="Enable Deep Inspect to retain layer and picture metadata."
          action={<Tool label="Enable Deep Inspect" onClick={() => void ws.run(() => core.setProfile("deep_inspect"))} />} />
      </Show>
    }>
      <Bar>
        <Search value={query()} change={setQuery} placeholder="Search views or source" controller={ws.bindSearch} />
        <Tool label={filter() ? filter().replace(/_/g, " ") : "All repaint paths"} onClick={() => setFilter(paths[(paths.indexOf(filter()) + 1) % paths.length])} />
      </Bar>
      <Divider />
      <SliceTable reset={`${query()}|${filter()}`} locate={locate()}
        empty={ws.live() ? "No views match the current snapshot" : "No view tree in this session"}
        query={(offset, limit) => { ws.repaint(); return core.queryViewTree(expanded(), query(), filter(), offset, limit); }}
        render={row => <TableRow row={row()} selected={selected() === row().id}
          onClick={() => selectedNode(row().id)}
          onHover={() => {
            if (!ws.live() || overlay().pickMode) return;
            if (hover !== undefined) clearTimeout(hover);
            hover = setTimeout(() => void ws.updateOverlay({ highlightNode: row().id }), 50);
          }}
          onLeave={() => { if (hover !== undefined) clearTimeout(hover); }}
          expanded={expanded().includes(row().id)}
          onExpand={() => setExpanded(current => current.includes(row().id) ? current.filter(id => id !== row().id) : [...current, row().id])}>
          <Show when={row().meta}><View style={{ width: 65, backgroundColor: c().elevated, padding: 2, borderRadius: 3 }}><Label subtle mono size={9}>{row().meta}</Label></View></Show>
          <Show when={row().count}><Label mono subtle size={10}>{row().count}</Label></Show>
        </TableRow>}
      />
    </Show>
  </PageLayout>;
}
