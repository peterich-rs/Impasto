import { For, Show, createMemo, createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { Canvas, Column, Image, Pressable, Row, View } from "@impasto/solid-ui";
import * as core from "@app/devtools-core";
import { Bar, Divider, Empty, KeyValues, Label, Pane, Search, Section, Spacer, Tabs, Tool, bytes, ms } from "../components/ui";
import { SliceTable, TableRow } from "../components/SliceTable";
import { InteractiveCanvas } from "../charts/InteractiveCanvas";
import { PageLayout } from "../shell/PageLayout";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";

const inspectorTabs = [{ id: "timing", label: "Timing" }, { id: "headers", label: "Headers" },
  { id: "body", label: "Payload" }, { id: "ws", label: "WS frames" }];
const kinds = [{ id: "", label: "All" }, { id: "http", label: "HTTP" }, { id: "socket", label: "WS" }, { id: "image", label: "Images" }];
const statusFilters = ["all", "pending", "2xx", "3xx", "4xx", "5xx", "failed", "error"];
const columnSizes = [65, 45, 0, 72, 54, 65, 74, 112];

function Waterfall(props: { segments: readonly number[] }) {
  const c = useTokens();
  return <Canvas style={{ width: 108, height: 12, flexShrink: 0, backgroundColor: c().elevated, borderRadius: 2 }} draw={(ctx, size) => {
    for (let i = 0; i < props.segments.length; i += 3) {
      ctx.fillStyle = c().buckets[props.segments[i + 2]];
      ctx.fillRect(props.segments[i] * size.width, 1, Math.max(1, props.segments[i + 1] * size.width), size.height - 2);
    }
  }} />;
}

function NetworkBody(props: { token: string; side: string; state?: string; height?: number }) {
  const ws = useWorkspace();
  const c = useTokens();
  const [body, setBody] = createSignal<core.BodySummary>();
  const [message, setMessage] = createSignal("");
  const [mode, setMode] = createSignal("raw");
  const [page, setPage] = createSignal(0);
  const [expanded, setExpanded] = createSignal<number[]>([]);
  const [search, setSearch] = createSignal("");
  createTrackedEffect(() => {
    const token = props.token;
    const side = props.side;
    ws.generation();
    ws.live();
    props.state;
    setBody(undefined);
    setPage(0);
    setExpanded([]);
    setMessage("");
    let active = true;
    void core.fetchNetworkBody(token, side).then(result => {
      if (!active) {
        releaseImage(result.imageSource);
        return;
      }
      setBody(result);
      setMode(result.mode === "text" ? "raw" : result.mode);
    }, error => { if (active) setMessage(String(error)); });
    return () => { active = false; };
  });
  function releaseImage(path?: string | null) {
    if (!path) return;
    core.releaseBodyImage(path);
  }
  createTrackedEffect(() => {
    const path = body()?.imageSource;
    onCleanup(() => releaseImage(path));
  });
  return <Column style={{ flex: 1, minHeight: 0, height: props.height }}>
    <Show when={body()} fallback={<Empty title={message() ? "Body unavailable" : "Loading body"} hint={message() || undefined} />}>
      {value => <Show when={value().available} fallback={<Empty title="Body unavailable"
        hint={value().evicted ? "The target evicted this body before it was retrieved." : "The recording contains no bytes for this body."} />}>
        <Bar>
          <Tabs items={[...(value().mode === "json" ? [{ id: "json", label: "JSON tree" }] : []),
            ...(value().mode === "image" ? [{ id: "image", label: "Image" }] : []),
            { id: "raw", label: "Raw" }, { id: "hex", label: "Hex" }]} selected={mode()} select={next => { setMode(next); setPage(0); }} />
          <Spacer /><Label subtle mono>{bytes(value().bytes)}</Label>
          <Tool label="Copy body" disabled={value().mode === "image" || value().mode === "hex"} onClick={() =>
            void ws.run(async () => ws.copy(await core.copyBodyText(props.token, props.side)))} />
        </Bar>
        <Show when={value().truncated || value().evicted}><Bar>
          <Label tone={c().warn} lines={2}>{value().evicted ? "Original body evicted · showing retained preview" : "Showing a truncated capture"}</Label>
        </Bar></Show>
        <Divider />
        <Show when={mode() === "json"} fallback={
          <Show when={mode() === "image" && value().imageSource} fallback={<>
            <Bar>
              <Tool label="Previous" disabled={page() <= 0} onClick={() => setPage(page() - 1024)} />
              <Tool label="Next" disabled={page() + 1024 >= value().bytes} onClick={() => setPage(page() + 1024)} />
              <Label mono subtle>{page()}–{Math.min(page() + 1024, value().bytes)}</Label>
            </Bar>
            <Pane><View style={{ padding: 10 }}>
              <Label mono size={10} lines={128}>{core.readBodyChunk(props.token, props.side, page(), 1024, mode() === "hex")}</Label>
            </View></Pane>
          </>}>
            <Section title="Captured image" detail={`${value().imageWidth} × ${value().imageHeight}`}>
              <Image source={`external:${value().imageSource}`} alt="Captured response"
                intrinsicSize={{ width: value().imageWidth ?? 1, height: value().imageHeight ?? 1 }}
                style={{ width: "100%", height: 220, objectFit: "contain" }} />
            </Section>
          </Show>
        }>
          <Bar><Search value={search()} change={setSearch} placeholder="Search keys and values" /></Bar>
          <SliceTable reset={search()} query={(offset, limit) => core.queryJsonRows(props.token, props.side, expanded(), search(), offset, limit)}
            render={row => <TableRow row={row()} expanded={expanded().includes(Number(row().id))}
              onExpand={() => setExpanded(items => {
                const index = Number(row().id);
                return items.includes(index) ? items.filter(item => item !== index) : [...items, index];
              })} />} />
        </Show>
      </Show>}
    </Show>
  </Column>;
}

function SocketFrames(props: { token: string }) {
  const ws = useWorkspace();
  const [direction, setDirection] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [selected, select] = createSignal("");
  const [revision, setRevision] = createSignal(0);
  const counters = createMemo(() => { ws.network(); return core.getNetworkDetail(props.token, "ws"); });
  const watermark = createMemo(() => `${props.token}:${counters().map(field => field.value).join(":")}`);
  createTrackedEffect(() => {
    watermark();
    const token = props.token;
    if (!ws.live()) return;
    let active = true;
    void core.refreshWsFrames(token).then(() => { if (active) setRevision(value => value + 1); },
      error => { if (active) ws.setNotice(String(error)); });
    return () => { active = false; };
  });
  createTrackedEffect(() => { props.token; select(""); });
  return <Column style={{ flex: 1, minHeight: 0 }}>
    <Section title="Socket traffic"><KeyValues fields={counters()} /></Section>
    <Bar>
      <Tool label={direction() || "Both directions"} onClick={() => setDirection(direction() === "" ? "in" : direction() === "in" ? "out" : "")} />
      <Search value={search()} change={setSearch} placeholder="Search frames" width={155} />
    </Bar>
    <Divider />
    <SliceTable reset={`${props.token}|${direction()}|${search()}`} query={(offset, limit) => {
      revision(); return core.queryWsFrames(props.token, direction(), search(), offset, limit);
    }} render={row => <TableRow row={row()} selected={selected() === row().id} onClick={() => select(row().id)}>
      <Label mono subtle size={10}>{bytes(row().count)}</Label>
    </TableRow>} />
    <Show when={selected()}>
      <Divider /><Bar><Label mono>{`Frame #${selected()}`}</Label><Spacer />
        <Tool label="Close frame body" icon="close" iconOnly onClick={() => select("")} /></Bar>
      <View style={{ height: 195, minHeight: 0, flexShrink: 0 }}><NetworkBody token={props.token} side={`ws:${selected()}`} /></View>
    </Show>
  </Column>;
}

export function NetworkPage() {
  const ws = useWorkspace();
  const c = useTokens();
  const [query, setQuery] = createSignal("");
  const [kind, setKind] = createSignal("");
  const [status, setStatus] = createSignal("all");
  const [sort, setSort] = createSignal("start");
  const [token, select] = createSignal("");
  const [isSocket, setIsSocket] = createSignal(false);
  const [tab, setTab] = createSignal("timing");
  const [hop, setHop] = createSignal<number>();
  const [side, setSide] = createSignal("response");
  const [summary, setSummary] = createSignal({ total: 0, bytes: 0, ttfb: 0 });
  createTrackedEffect(() => {
    const intent = ws.intent();
    if (intent.kind !== "network") return;
    if (intent.token) select(intent.token);
    setStatus(intent.status ?? "all");
    setTab(intent.tab ?? "timing");
  });
  function pick(entry: core.NetworkRow) {
    select(entry.id);
    setHop(undefined);
    setSide("response");
    setIsSocket(entry.kind === "socket");
    if (entry.kind !== "socket" && tab() === "ws") setTab("timing");
    ws.showInspector(true);
  }
  const details = (section: string) => { ws.network(); return token() ? core.getNetworkDetail(token(), section) : []; };
  const responseState = createMemo(() => details("general").find(field => field.name === "state")?.value);
  return <PageLayout title="Network" inspectorTitle="Request inspector" actions={<Label mono subtle>{summary().total} requests · {bytes(summary().bytes)}</Label>}
    inspector={<Show when={token()} fallback={<Empty title="Select a request" hint="Inspect the connection, redirect hops, redacted headers and captured payload." />}>
      <Column style={{ flex: 1, minHeight: 0 }}>
        <Bar><Tabs items={inspectorTabs.filter(item => item.id !== "ws" || isSocket())} selected={tab()} select={setTab} /></Bar>
        <Divider />
        <Show when={tab() === "ws"} fallback={
          <Show when={tab() === "body"} fallback={<Pane>
            <Section title={`Request #${token()}`}>
              <KeyValues fields={details("general")} />
              <Tool label="Copy as cURL" onClick={() => void ws.run(() => ws.copy(core.formatCurl(token())))} />
            </Section>
            <Divider />
            <Show when={tab() === "headers"} fallback={<>
              <Section title="Timing" detail="Milliseconds">
                <InteractiveCanvas label="Network waterfall" height={162} project={(w, h) => {
                  ws.network(); return core.projectNetworkWaterfall(token(), hop() ?? null, w, h);
                }} />
                <For each={(ws.network(), core.queryRedirects(token(), 0, 24).rows)} keyed={row => row.id}>{row =>
                  <Tool label={`Hop ${Number(row().id) + 1} · ${row().kind}`} active={hop() === Number(row().id)} onClick={() => setHop(current => current === Number(row().id) ? undefined : Number(row().id))} />
                }</For>
                <Show when={hop() != null}><Label mono subtle lines={4}>{core.queryRedirects(token(), hop() ?? 0, 1).rows[0]?.detail}</Label></Show>
              </Section>
              <Divider /><Section title="Connection"><KeyValues fields={details("connection")} /></Section>
            </>}>
              <Section title="Request headers"><KeyValues fields={details("request_headers")} /></Section>
              <Divider />
              <Section title="Response headers"><KeyValues fields={details("response_headers")} /></Section>
            </Show>
          </Pane>}>
            <Bar><Tabs items={[{ id: "request", label: "Request" }, { id: "response", label: "Response" }]} selected={side()} select={setSide} /></Bar>
            <NetworkBody token={token()} side={side()} state={responseState()} />
          </Show>
        }><SocketFrames token={token()} /></Show>
      </Column>
    </Show>}>
    <Bar>
      <Search value={query()} change={setQuery} placeholder="Filter URL, headers or body" controller={ws.bindSearch} />
      <Tabs items={kinds} selected={kind()} select={setKind} />
      <Tool label={status() === "all" ? "All statuses" : status()} onClick={() => setStatus(statusFilters[(statusFilters.indexOf(status()) + 1) % statusFilters.length])} />
      <Spacer />
      <Label mono size={11} subtle>Mean TTFB {ms(summary().ttfb || null)}</Label>
    </Bar>
    <Divider />
    <Row style={{ backgroundColor: c().panel, height: 30, paddingLeft: 8, paddingRight: 8, alignItems: "center", gap: 4 }}>
      <For each={["Status", "Verb", "Name / Path", "Initiator", "Protocol", "Size", "Time", "Waterfall"]}>{(label, index) =>
        <View style={{ width: columnSizes[index()] || undefined, flex: index() === 2 ? 1 : undefined, minWidth: 0 }}>
          <Show when={[0, 2, 5, 6].includes(index())} fallback={<Label subtle size={11}>{label}</Label>}>
            <Tool label={label} active={index() === 0 ? sort() === "status" : index() === 5 ? sort() === "size" : index() === 6 ? sort() === "time" : sort() === "start"}
              onClick={() => setSort(index() === 0 ? "status" : index() === 5 ? "size" : index() === 6 ? "time" : "start")} />
          </Show>
        </View>
      }</For>
    </Row>
    <Divider />
    <SliceTable reset={`${query()}|${kind()}|${status()}|${sort()}`} query={(offset, limit) => {
      ws.network();
      const result = core.queryNetworkSlice(query(), kind(), status(), sort(), offset, limit);
      queueMicrotask(() => setSummary({ total: result.total, bytes: result.bytes, ttfb: result.meanTtfb }));
      return result;
    }} render={entry =>
      <Pressable role={1} accessibilityLabel={entry().url} onClick={() => pick(entry())}
        style={{ height: 26, paddingLeft: 8, paddingRight: 8, flexDirection: "row", alignItems: "center", gap: 4,
          backgroundColor: token() === entry().id ? c().selected : c().canvas, cursor: "pointer" }}>
        <For each={[
          entry().status, entry().method || "—", entry().url || "—", entry().initiator || "—", entry().protocol ? (entry().reused ? `${entry().protocol} ↻` : entry().protocol) : "—",
          bytes(entry().bytes), ms(entry().elapsedMs),
        ]} keyed={false}>{(value, index) => <View style={{ width: columnSizes[index] || undefined, flex: index === 2 ? 1 : undefined, minWidth: 0 }}>
          <Label size={11} mono={index !== 2} subtle={index > 2}
            tone={index === 0 ? (/^[45]|failed/.test(value()) ? c().danger : value().startsWith("2") ? c().ok : c().warn) : undefined}>{value()}</Label>
        </View>}</For>
        <Waterfall segments={entry().segments} />
      </Pressable>
    } />
  </PageLayout>;
}
