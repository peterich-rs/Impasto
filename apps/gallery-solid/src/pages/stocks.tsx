import { For, Show, createMemo, createSignal, onCleanup, untrack } from "solid-js";
import { onFrame, onLayout, requestFrame, scene } from "@impasto/solid-renderer";
import { Canvas, Column, Pressable, Role, Row, View, VirtualList, createScrollController } from "@impasto/solid-ui";
import type { Router } from "@impasto/solid-router";
import { store as devtools } from "@impasto/devtools";
import { fmtPrice } from "../format.ts";
import { loc } from "../copy.ts";
import { marketOpen, minute, sectorOf, sectors } from "../engine/analytics.ts";
import { downsampleTicks } from "../engine/universe.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Instrument } from "../engine/types.ts";
import { useMarketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { dot, stroke } from "../widgets/stocks/chart.tsx";
import { Action, change, ClockButton, clock, code, colors, filling, Label, Rule, sectorLabel, Segments, sign, tone } from "../widgets/stocks/ui.tsx";
import { StockView } from "./stock.tsx";

const filters = ["all", "gainers", "losers", "active"] as const;
type Filter = typeof filters[number];
const initialIndex = 4120.0;
const publicationGroups = 8;

/** Distributes the market snapshots across frames while keeping the per-row cadence. */
function useRowPublications(revision: () => number, active: () => boolean) {
  let published = untrack(revision);
  let group = publicationGroups;
  const groups = Array.from({ length: publicationGroups }, () => createSignal(published));
  onCleanup(onFrame(() => {
    const next = revision();
    if (!active()) {
      group = publicationGroups;
      return;
    }
    if (next !== published && group === publicationGroups) {
      published = next;
      group = 0;
    }
    const current = groups[group];
    if (current) {
      current[1](published);
      group++;
      if (group < publicationGroups) requestFrame();
    }
  }));
  return (index: number) => groups[index % publicationGroups]![0]();
}

function filterLabel(filter: Filter): string {
  switch (filter) {
    case "all": return loc("全部", "All");
    case "gainers": return loc("上涨", "Gainers");
    case "losers": return loc("下跌", "Losers");
    case "active": return loc("活跃", "Active");
  }
}

function marketSummary(engine: QuoteEngine, instruments: readonly Instrument[]) {
  let total = 0;
  let up = 0;
  let down = 0;
  const groups = sectors.map(() => ({ sum: 0, count: 0 }));
  for (const instrument of instruments) {
    const value = change(engine.quote(instrument.symbol));
    if (value > 0) up++;
    if (value < 0) down++;
    total += value;
    const sector = sectorOf(instrument);
    const group = sector ? groups[sectors.indexOf(sector)] : undefined;
    if (group) { group.sum += value; group.count++; }
  }
  return { up, down, flat: instruments.length - up - down, change: total / Math.max(1, instruments.length),
    groups: groups.map(({ sum, count }) => count === 0 ? undefined : sum / count) };
}

function MarketStrip(props: { time: number; summary: ReturnType<typeof marketSummary>; count: number; router: Router; selected: Filter; onSelect: (value: Filter) => void }) {
  const [expanded, setExpanded] = createSignal(false);
  const [menu, setMenu] = createSignal(false);
  const scale = () => Math.max(0.5, ...props.summary.groups.map((value) => Math.abs(value ?? 0))) * 1.1;
  const width = (value: number) => `${value * 100 / Math.max(1, props.count)}%` as const;
  return <Column style={{ gap: 8, padding: 12, flexShrink: 0 }}>
    <Row style={{ gap: 8, alignItems: "center" }}>
      <Pressable role={Role.Button} onClick={() => setExpanded((value) => !value)}
        accessibilityLabel={loc("展开综合指数", "Toggle composite breadth")}
        style={{ flex: 1, minWidth: 0, minHeight: 34, gap: 7, alignItems: "center", flexDirection: "row" }}>
        <Label size={12} color={colors.dim}>{loc("综合指数", "Composite")}</Label>
        <Label size={16} weight={600}>{fmtPrice(initialIndex * (1 + props.summary.change / 100))}</Label>
        <Label size={12} color={tone(props.summary.change)}>{`${sign(props.summary.change)}%`}</Label>
        <Label size={12} color={colors.faint}>{expanded() ? "⌃" : "⌄"}</Label>
      </Pressable>
      <Label size={11} color={colors.faint}>{clock(props.time)}</Label>
      <ClockButton />
    </Row>
    <Show when={expanded()}>
      <Column style={{ gap: 8, paddingBottom: 6 }}>
        <Row style={{ height: 6, borderRadius: 3, overflow: "hidden" }}>
          <View style={{ height: 6, width: width(props.summary.up), backgroundColor: tone(1) }} />
          <View style={{ height: 6, width: width(props.summary.flat), backgroundColor: colors.faint }} />
          <View style={{ height: 6, width: width(props.summary.down), backgroundColor: tone(-1) }} />
        </Row>
        <Row style={{ gap: 6, justifyContent: "space-between" }}>
          <Label size={12} color={tone(1)}>{loc(`${props.summary.up} 上涨`, `${props.summary.up} up`)}</Label>
          <Label size={12} color={colors.faint}>{loc(`${props.summary.flat} 平 · ${props.count} 只`, `${props.summary.flat} flat · ${props.count} issues`)}</Label>
          <Label size={12} color={tone(-1)}>{loc(`${props.summary.down} 下跌`, `${props.summary.down} down`)}</Label>
        </Row>
        <For each={sectors}>{(sector, index) => {
          const value = () => props.summary.groups[index()];
          const bar = (positive: boolean) => `${value() !== undefined && (value()! > 0) === positive ? Math.abs(value()!) * 100 / scale() : 0}%` as const;
          return <Row style={{ height: 18, gap: 8, alignItems: "center" }}>
            <Label size={12} color={colors.dim} style={{ width: 80 }}>{sectorLabel(sector)}</Label>
            <Row style={{ flex: 1, gap: 1, alignItems: "center" }}>
              <View style={{ flex: 1, alignItems: "end" }}><View style={{ width: bar(false), height: 7, borderRadius: 2, backgroundColor: tone(-1) }} /></View>
              <View style={{ width: 1, height: 13, backgroundColor: colors.line }} />
              <View style={{ flex: 1 }}><View style={{ width: bar(true), height: 7, borderRadius: 2, backgroundColor: tone(1) }} /></View>
            </Row>
            <Label size={12} color={tone(value() ?? 0)} style={{ width: 54, textAlign: "right" }}>{value() === undefined ? "—" : `${sign(value()!)}%`}</Label>
          </Row>;
        }}</For>
      </Column>
    </Show>
    <Row style={{ gap: 5, alignItems: "center", justifyContent: "space-between" }}>
      <Segments options={filters} selected={props.selected} label={filterLabel} onSelect={props.onSelect}
        count={(filter) => filter === "all" ? props.count : filter === "gainers" ? props.summary.up : filter === "losers" ? props.summary.down : undefined} />
      <Action label="⋯" onClick={() => setMenu((value) => !value)} title={loc("更多页面", "More pages")} style={{ minHeight: 28, backgroundColor: "#00000000", padding: 6 }} />
    </Row>
    <Show when={menu()}>
      <Row style={{ gap: 5, flexWrap: "wrap" }}>
        <Action label={loc("搜索", "Search")} onClick={() => props.router.push("/search")} />
        <Action label={loc("自选", "Watchlist")} onClick={() => props.router.reset("/watchlist")} />
        <Action label={loc("市场", "Market")} onClick={() => props.router.reset("/market")} />
        <Action label={loc("交易", "Trades")} onClick={() => props.router.reset("/trade")} />
        <Action label={loc("我的", "Profile")} onClick={() => props.router.reset("/profile")} />
        <Show when={devtools.available()}><Action label={loc("调试", "Devtools")} onClick={() => devtools.open()} /></Show>
      </Row>
    </Show>
  </Column>;
}

function Spark(props: { engine: QuoteEngine; symbol: string; revision: number }) {
  return <Canvas deferWhenOffscreen style={{ width: 96, maxWidth: "34%", flexShrink: 1, height: 36 }} draw={(ctx, { width, height }) => {
    props.revision;
    const prices = downsampleTicks(props.engine.ring(props.symbol), 24);
    if (!prices.length || width < 18) return;
    const quote = props.engine.quote(props.symbol);
    const color = tone(quote.last - quote.prevClose);
    const min = Math.min(...prices);
    const max = Math.max(...prices);
    const span = Math.max(0.01, max - min);
    const y = (price: number) => height - 6 - (price - min + (max - min < 0.01 ? span / 2 : 0)) / span * (height - 12);
    const plot = width - 9;
    const x = (index: number) => 1 + index * (plot - 1) / (prices.length - 1);
    const curve = () => {
      ctx.beginPath();
      ctx.moveTo(x(0), y(prices[0]!));
      for (let index = 1; index < prices.length - 1; index++) {
        ctx.quadraticCurveTo(x(index), y(prices[index]!), (x(index) + x(index + 1)) / 2, (y(prices[index]!) + y(prices[index + 1]!)) / 2);
      }
      ctx.lineTo(plot, y(prices.at(-1)!));
    };
    ctx.setLineDash([1, 3]);
    stroke(ctx, colors.faint, 0, y(prices[0]!), plot, y(prices[0]!));
    ctx.setLineDash([]);
    curve();
    ctx.lineTo(plot, height);
    ctx.lineTo(0, height);
    ctx.closePath();
    const gradient = ctx.createLinearGradient(0, 0, 0, height);
    gradient.addColorStop(0, `${color}35`);
    gradient.addColorStop(1, `${color}00`);
    ctx.fillStyle = gradient;
    ctx.fill();
    curve();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.4;
    ctx.lineCap = "round";
    ctx.stroke();
    dot(ctx, `${color}25`, plot, y(prices.at(-1)!), 7);
    dot(ctx, color, plot, y(prices.at(-1)!), 2.7);
  }} />;
}

function StockRow(props: { instrument: Instrument; engine: QuoteEngine; revision: number; sparkRevision: number; selected: boolean; onClick: () => void }) {
  const quote = createMemo(() => { props.revision; return { ...props.engine.quote(props.instrument.symbol) }; });
  const percent = createMemo(() => change(quote()));
  const color = createMemo(() => tone(percent()));
  return <Column style={{ height: 61, gap: 0 }}>
    <Pressable role={Role.Button} onClick={props.onClick} accessibilityLabel={`${code(props.instrument.symbol)} ${props.instrument.name}`}
      style={{ flex: 1, minHeight: 0, paddingLeft: 14, paddingRight: 14, flexDirection: "row", gap: 8, alignItems: "center",
        backgroundColor: props.selected ? colors.raised : colors.bg }}>
      <Show when={props.selected}><View style={{ position: "absolute", left: 0, top: 12, bottom: 12, width: 3, borderRadius: 2, backgroundColor: colors.lime }} /></Show>
      <Column style={{ flex: 1, minWidth: 0, gap: 3 }}>
        <Label size={15} weight={600}>{code(props.instrument.symbol)}</Label>
        <Label size={12} color={colors.dim}>{props.instrument.name}</Label>
      </Column>
      <Spark engine={props.engine} symbol={props.instrument.symbol} revision={props.sparkRevision} />
      <Column style={{ minWidth: 65, alignItems: "end", gap: 3 }}>
        <Label size={15} weight={600}>{fmtPrice(quote().last)}</Label>
        <Label size={12} color={color()}>{`${sign(percent())}%`}</Label>
      </Column>
    </Pressable>
    <Rule />
  </Column>;
}

export function StocksPage(props: { router: Router }) {
  const { engine } = useQuotes();
  const instruments = engine.instruments().filter((instrument) => !instrument.index);
  const [width, setWidth] = createSignal(scene.metrics().width);
  onCleanup(onLayout(() => setWidth(scene.metrics().width)));
  const [filter, setFilter] = createSignal<Filter>("all");
  const [selected, setSelected] = createSignal(instruments[0]!.symbol);
  const controller = createScrollController();
  const wide = () => width() >= 800;
  const split = () => width() >= 1240;
  const canPublish = () => {
    const top = props.router.stack().at(-1);
    return top?.path === "/stocks" && top.phase === "live";
  };
  const revision = useMarketPulse(canPublish);
  const rowRevision = useRowPublications(revision, canPublish);
  const publicationIndex = new Map(instruments.map(({ symbol }, index) => [symbol, index]));
  const overviewRevision = createMemo(() => Math.floor(revision() / 2));
  const time = createMemo(() => { overviewRevision(); return engine.simNow() - engine.dayStart(); });
  const rankAt = createMemo(() => Math.floor(revision() / 5));
  const summary = createMemo(() => { overviewRevision(); return marketSummary(engine, instruments); });
  const items = createMemo<readonly Instrument[]>((previous) => {
    const choice = filter();
    if (choice === "all") return instruments;
    rankAt();
    const percent = (instrument: Instrument) => change(engine.quote(instrument.symbol));
    const turnover = (instrument: Instrument) => engine.analytics(instrument.symbol).turnover;
    const ranked = choice === "active" ? [...instruments].sort((a, b) => turnover(b) - turnover(a)).slice(0, 10)
      : instruments.filter((instrument) => choice === "gainers" ? percent(instrument) > 0 : percent(instrument) < 0)
        .sort((a, b) => choice === "gainers" ? percent(b) - percent(a) : percent(a) - percent(b));
    return previous && previous.length === ranked.length && ranked.every((item, index) => item === previous[index]) ? previous : ranked;
  });
  return <Row style={[filling, { backgroundColor: colors.bg }]}>
    <Column style={[filling, wide() ? { flex: 0, width: "28%", minWidth: 280, maxWidth: 380 } : { width: "100%" }]}>
      <MarketStrip time={marketOpen * minute + time()} summary={summary()} count={instruments.length} router={props.router} selected={filter()}
        onSelect={(value) => { if (filter() !== value) { setFilter(value); controller.scrollTo(0, 0, false); } }} />
      <Rule />
      <VirtualList items={items()} itemKey={(instrument) => instrument.symbol} controller={controller} estimatedHeight={61} overscan={8}
        style={filling} empty={() => <Label color={colors.dim} style={{ padding: 20, textAlign: "center" }}>{loc("暂无符合条件的证券", "Nothing here right now")}</Label>}
        recycle={{
          typeOf: () => "stock",
          poolSize: 16,
          render: (instrument) => {
            const index = () => publicationIndex.get(instrument().symbol)!;
            const row = () => rowRevision(index());
            const spark = createMemo(() => Math.floor((row() + index() % 3) / 3));
            return <StockRow instrument={instrument()} engine={engine} revision={row()} sparkRevision={spark()} selected={wide() && selected() === instrument().symbol}
              onClick={() => {
                const symbol = instrument().symbol;
                setSelected(symbol);
                if (!wide()) props.router.push(`/stock/${symbol}`);
              }} />;
          },
        }} />
    </Column>
    <Show when={wide()}>
      <Rule vertical />
      <StockView engine={engine} symbol={selected()} wide split={split()} />
    </Show>
  </Row>;
}
