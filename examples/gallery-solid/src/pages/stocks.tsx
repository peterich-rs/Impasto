import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";
import { onLayout, scene } from "@impasto/solid-renderer";
import { Canvas, Column, Pressable, Role, Row, View, VirtualList, createScrollController } from "@impasto/solid-ui";
import { usePageActive, type Router } from "@impasto/solid-router";
import { fmtPrice, fmtSigned } from "../format.ts";
import { loc } from "../copy.ts";
import { marketOpen, minute, sectorOf, sectors } from "../engine/analytics.ts";
import { downsampleTicks } from "../engine/universe.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Instrument, Quote } from "../engine/types.ts";
import { useMarketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { inWatchlist, toggleWatch, watchlistSig } from "../state/watchlist.ts";
import { dot, stroke } from "../widgets/stocks/chart.tsx";
import { Action, change, ClockButton, clock, code, colors, filling, Label, PriceTag, Rule, sectorLabel, Segments, sign, tone } from "../widgets/stocks/ui.tsx";
import { Glyph, Icon } from "../widgets/stocks/icons.tsx";

type StockView = typeof import("./stock.tsx").StockView;

function InlineStock(props: Parameters<StockView>[0]) {
  const [view, setView] = createSignal<StockView>();
  const [fault, setFault] = createSignal<{ error: unknown }>();
  let mounted = true;
  onCleanup(() => { mounted = false; });
  void import("./stock.tsx").then(
    ({ StockView }) => { if (mounted) setView(() => StockView); },
    error => { if (mounted) setFault({ error }); },
  );
  return () => {
    const failed = fault();
    if (failed) throw failed.error;
    const StockView = view();
    return StockView ? <StockView {...props} /> : 0;
  };
}

const filters = ["all", "gainers", "losers", "active"] as const;
type Filter = typeof filters[number];
const initialIndex = 4120.0;
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

function BoardHeader(props: { time: number; summary: ReturnType<typeof marketSummary>; count: number; router: Router; selected: Filter; onSelect: (value: Filter) => void; onSearch: () => void }) {
  const [expanded, setExpanded] = createSignal(false);
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
        <Icon name={expanded() ? "chevronUp" : "chevronDown"} size={16} color={colors.faint} />
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
      <Glyph name="search" title={loc("搜索", "Search")} onClick={props.onSearch} size={18} color={colors.dim} compact />
    </Row>
  </Column>;
}

function Spark(props: { prices: readonly number[]; color: string }) {
  return <Canvas style={{ width: 96, maxWidth: "26%", minWidth: 0, flexShrink: 4, height: 36 }} draw={(ctx, { width, height }) => {
    const prices = props.prices;
    if (prices.length < 2 || width < 18) return;
    const color = props.color;
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

/**
 * Two-band row shared by the market and watchlist boards. Upper band:
 * identity, spark, price badge. Lower band: change and percent on one line.
 */
function StockRow(props: { instrument: Instrument; engine: QuoteEngine; revision: number; selected: boolean; onClick: () => void; saved?: boolean; onSave?: () => void }) {
  const quote = createMemo<Pick<Quote, "last" | "prevClose">>((previous) => {
    props.revision;
    const { last, prevClose } = props.engine.quote(props.instrument.symbol);
    return previous?.last === last && previous.prevClose === prevClose
      ? previous : { last, prevClose };
  });
  const prices = createMemo((previous?: readonly number[]) => {
    props.revision;
    const next = downsampleTicks(props.engine.ring(props.instrument.symbol), 24);
    return previous?.length === next.length && next.every((price, index) => price === previous[index])
      ? previous : next;
  });
  const percent = createMemo(() => change(quote()));
  const color = createMemo(() => tone(percent()));
  return <Column style={{ height: 72, gap: 0 }}>
    <Pressable role={Role.Button} onClick={props.onClick} accessibilityLabel={`${code(props.instrument.symbol)} ${props.instrument.name}`}
      style={{ flex: 1, minHeight: 0, paddingLeft: 14, paddingRight: 14, paddingTop: 10, flexDirection: "column", gap: 4,
        backgroundColor: props.selected ? colors.raised : colors.bg }}>
      <Show when={props.selected}><View style={{ position: "absolute", left: 0, top: 12, bottom: 12, width: 3, borderRadius: 2, backgroundColor: colors.lime }} /></Show>
      <Row style={{ flex: 1, minHeight: 0, minWidth: 0, gap: 8, alignItems: "center" }}>
        <Column style={{ flex: 1, minWidth: 96, gap: 3 }}>
          <Row style={{ gap: 6, alignItems: "baseline", minWidth: 0 }}>
            <Label size={15} weight={600} style={{ flexShrink: 0 }}>{code(props.instrument.symbol)}</Label>
            <Label size={12} color={colors.dim} style={{ flex: 1 }}>{props.instrument.name}</Label>
          </Row>
          <Row style={{ gap: 10, alignItems: "baseline" }}>
            <Label size={12} color={color()}>{fmtSigned(quote().last - quote().prevClose)}</Label>
            <Label size={12} color={color()}>{`${sign(percent())}%`}</Label>
          </Row>
        </Column>
        <Show when={props.onSave}><Glyph name={props.saved ? "starFilled" : "star"} title={props.saved ? loc("移出自选", "Remove from watchlist") : loc("加入自选", "Add to watchlist")}
          onClick={props.onSave!} size={18} color={props.saved ? colors.amber : colors.faint} compact style={{ backgroundColor: "#00000000" }} /></Show>
        <Spark prices={prices()} color={color()} />
        <PriceTag value={fmtPrice(quote().last)} change={percent()} />
      </Row>
    </Pressable>
    <Rule />
  </Column>;
}

export type BoardProps = { router: Router; title: string; empty: string; emptyAction?: string; onEmpty?: () => void };

/** Market board over the full universe. */
export function MarketsPage(props: BoardProps) {
  const { engine } = useQuotes();
  return <QuoteBoard {...props} engine={engine} instruments={() => engine.instruments().filter((instrument) => !instrument.index)} scrollKey="markets" />;
}

/** Watchlist board: same rows and cadence, sourced from the saved symbols. */
export function WatchlistPage(props: BoardProps) {
  const { engine } = useQuotes();
  // Indices stay out of the list; the board header already shows the composite.
  const bySymbol = new Map(engine.instruments()
    .filter((instrument) => !instrument.index)
    .map((instrument) => [instrument.symbol, instrument]));
  const instruments = createMemo(() => watchlistSig().flatMap((symbol) => {
    const instrument = bySymbol.get(symbol);
    return instrument ? [instrument] : [];
  }));
  return <QuoteBoard {...props} engine={engine} instruments={instruments} scrollKey="watchlist" savable />;
}

function QuoteBoard(props: BoardProps & { engine: QuoteEngine; instruments: () => readonly Instrument[]; scrollKey: string; savable?: boolean }) {
  const instruments = () => props.instruments();
  const [width, setWidth] = createSignal(scene.metrics().width);
  onCleanup(onLayout(() => setWidth(scene.metrics().width)));
  const [filter, setFilter] = createSignal<Filter>("all");
  const [selected, setSelected] = createSignal<string | null>(null);
  const controller = createScrollController();
  const wide = () => width() >= 800;
  const split = () => width() >= 1240;
  const revision = useMarketPulse(usePageActive());
  const time = createMemo(() => { revision(); return engineTime(props.engine); });
  const summary = createMemo(() => { revision(); return marketSummary(props.engine, instruments()); });
  const items = createMemo<readonly Instrument[]>((previous) => {
    const source = instruments();
    const choice = filter();
    if (choice === "all") return source;
    revision();
    const percent = (instrument: Instrument) => change(props.engine.quote(instrument.symbol));
    const turnover = (instrument: Instrument) => props.engine.analytics(instrument.symbol).turnover;
    const ranked = choice === "active" ? [...source].sort((a, b) => turnover(b) - turnover(a)).slice(0, 10)
      : source.filter((instrument) => choice === "gainers" ? percent(instrument) > 0 : percent(instrument) < 0)
        .sort((a, b) => choice === "gainers" ? percent(b) - percent(a) : percent(a) - percent(b));
    return previous && previous.length === ranked.length && ranked.every((item, index) => item === previous[index]) ? previous : ranked;
  });
  const current = () => (selected() ?? instruments()[0]?.symbol ?? "") as string;
  return <Row style={[filling, { backgroundColor: colors.bg }]}>
    <Column style={[filling, wide() ? { flex: 0, width: "28%", minWidth: 280, maxWidth: 380 } : { width: "100%" }]}>
      <BoardHeader time={time()} summary={summary()} count={instruments().length} router={props.router} selected={filter()}
        onSearch={() => props.router.push("/search")}
        onSelect={(value) => { if (filter() !== value) { setFilter(value); controller.scrollTo(0, 0); } }} />
      <Rule />
      <VirtualList items={items()} itemKey={(instrument) => instrument.symbol} controller={controller} estimatedHeight={72} overscan={8}
        restoreKey={props.scrollKey}
        style={filling} empty={() => <Column style={{ gap: 12, padding: 24, alignItems: "center" }}>
          <Label color={colors.dim} style={{ textAlign: "center" }}>{props.empty}</Label>
          <Show when={props.onEmpty}><Action label={props.emptyAction ?? loc("去搜索", "Search")} onClick={props.onEmpty!} primary /></Show>
        </Column>}
        recycle={{
          typeOf: () => "stock",
          poolSize: 16,
          render: (instrument) => {
            return <StockRow instrument={instrument()} engine={props.engine} revision={revision()}
              selected={wide() && current() === instrument().symbol}
              saved={props.savable && inWatchlist(instrument().symbol)}
              onSave={props.savable ? () => toggleWatch(instrument().symbol) : undefined}
              onClick={() => {
                const symbol = instrument().symbol;
                setSelected(symbol);
                if (!wide()) props.router.push(`/stock/${symbol}`);
              }} />;
          },
        }} />
      {/* The route capsule floats over this strip; keep the last row clear of it. */}
      <Show when={!wide()}><View style={{ height: 96, flexShrink: 0 }} /></Show>
    </Column>
    <Show when={wide()}>
      <Rule vertical />
      <InlineStock engine={props.engine} symbol={current()} wide split={split()} />
    </Show>
  </Row>;
}

function engineTime(engine: QuoteEngine): number {
  return marketOpen * minute + engine.simNow() - engine.dayStart();
}
