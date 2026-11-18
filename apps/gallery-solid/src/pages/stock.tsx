import { For, Show, createMemo, createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { Canvas, Column, Row, ScrollArea, View } from "@impasto/solid-ui";
import type { Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { fmtPrice, fmtSigned } from "../format.ts";
import { book, marketOpen, minute, sectorOf } from "../engine/analytics.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Quote, OrderSide, Tick } from "../engine/types.ts";
import { marketRunning, useMarketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { inWatchlist, toggleWatch } from "../state/watchlist.ts";
import { Flow, historicalRange, Ranges, Tape } from "../widgets/stocks/analysis.tsx";
import { dot, StockChart, stroke } from "../widgets/stocks/chart.tsx";
import { Depth, OrderBook } from "../widgets/stocks/order-book.tsx";
import { Action, change, ClockButton, clock, code, colors, compact, filling, Label, Rule, Section, sectorLabel, sign, tone } from "../widgets/stocks/ui.tsx";
import { TicketOverlay, type TicketSeed } from "./order.tsx";

function QuoteHeader(props: { engine: QuoteEngine; quote: Quote; wide: boolean }) {
  const quote = () => props.quote;
  const instrument = () => props.engine.ins(quote().symbol);
  const color = () => tone(quote().last - quote().prevClose);
  const stats = [0, 1, 2, 3, 4, 5, 6, 7];
  const labels = () => [loc("今开", "Open"), loc("昨收", "Prev close"), loc("均价", "Avg price"), loc("振幅", "Amplitude"),
    loc("成交量", "Volume"), loc("成交额", "Turnover"), loc("120 日高", "120d high"), loc("120 日低", "120d low")];
  const values = createMemo(() => {
    const q = quote();
    const { turnover } = props.engine.analytics(q.symbol);
    const [low, high] = historicalRange(props.engine.bars(q.symbol), q, 120);
    return [fmtPrice(q.open), fmtPrice(q.prevClose), fmtPrice(q.volume ? turnover / q.volume : q.open),
      `${((q.high - q.low) / q.prevClose * 100).toFixed(2)}%`, compact(q.volume), compact(turnover), fmtPrice(high), fmtPrice(low)];
  });
  return <Section>
    <Show when={props.wide}>
      <Row style={{ alignItems: "center", justifyContent: "space-between" }}>
        <Label size={23} weight={600}>{code(quote().symbol)}</Label>
        <Favorite symbol={quote().symbol} />
      </Row>
    </Show>
    <Row style={{ gap: 9, alignItems: "center" }}>
      <Label size={15} color={colors.dim} style={{ flex: 1 }}>{instrument().name}</Label>
      <Label size={12} color={colors.faint}>{sectorLabel(sectorOf(instrument()))}</Label>
      <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: marketRunning() ? colors.lime : colors.faint }} />
      <Label size={11} color={colors.dim}>{clock(marketOpen * minute + props.engine.simNow() - quote().dayStart)}</Label>
    </Row>
    <Row style={{ gap: 8, alignItems: "center", justifyContent: "space-between" }}>
      <Row style={{ gap: 6, alignItems: "center", minWidth: 0, flexShrink: 1 }}>
        <Label size={props.wide ? 38 : 36} weight={600} color={color()}>{fmtPrice(quote().last)}</Label>
        <Label size={19} color={color()}>{quote().last > quote().prevClose ? "↑" : quote().last < quote().prevClose ? "↓" : "—"}</Label>
      </Row>
      <Column style={{ gap: 3, alignItems: "end" }}>
        <Label size={17} weight={600} color={color()}>{`${sign(change(quote()))}%`}</Label>
        <Label size={12} color={color()}>{loc(`${fmtSigned(quote().last - quote().prevClose)} 今日`, `${fmtSigned(quote().last - quote().prevClose)} today`)}</Label>
      </Column>
    </Row>
    <Row style={{ alignItems: "center", gap: 10, height: 13 }}>
      <Label size={11} color={colors.dim}>{`L ${fmtPrice(quote().low)}`}</Label>
      <Canvas style={{ height: 13, flex: 1, minWidth: 12 }} draw={(ctx, { width }) => {
        const q = quote();
        const share = (value: number) => q.high === q.low ? 0.5 : Math.max(0, Math.min(1, (value - q.low) / (q.high - q.low)));
        const x = (value: number) => 4 + Math.max(0, width - 8) * share(value);
        ctx.lineWidth = 3;
        ctx.lineCap = "round";
        stroke(ctx, colors.line, 3, 6, width - 3, 6);
        ctx.lineWidth = 1.5;
        stroke(ctx, colors.dim, x(q.prevClose), 2, x(q.prevClose), 10);
        dot(ctx, colors.bg, x(q.last), 6, 5);
        dot(ctx, color(), x(q.last), 6, 3.5);
      }} />
      <Label size={11} color={colors.dim}>{`H ${fmtPrice(quote().high)}`}</Label>
    </Row>
    <View style={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0px, 1fr))", columnGap: 8, rowGap: 9 }}>
      <For each={stats}>{(index) => <Column style={{ gap: 4, minWidth: 0 }}>
        <Label color={colors.dim} size={11}>{labels()[index]}</Label>
        <Label size={14}>{values()[index]}</Label>
      </Column>}</For>
    </View>
  </Section>;
}

function Favorite(props: { symbol: string }) {
  const saved = () => inWatchlist(props.symbol);
  return <Action label={saved() ? "★" : "☆"} title={saved() ? loc("移出自选", "Remove from watchlist") : loc("加入自选", "Add to watchlist")}
    onClick={() => toggleWatch(props.symbol)} style={{ backgroundColor: "#00000000" }} />;
}

export function StockView(props: { engine: QuoteEngine; symbol: string; wide?: boolean; split?: boolean; transitioning?: boolean }) {
  const sections = ["quote", "chart", "book", "tape", "depth", "flow", "ranges"] as const;
  const [ready, setReady] = createSignal(-1);
  const [ticket, setTicket] = createSignal<TicketSeed | null>(null);
  let stage = -1;
  // Give the route and each section separate turns on the render thread.
  const stop = onFrame(() => {
    // Mount the quote and chart on entry. The sections below them wait until the slide ends.
    if ((!props.transitioning && ticket() === null) || stage < sections.indexOf("chart")) setReady(++stage);
    if (stage === sections.length - 1) stop();
    else requestFrame();
  });
  onCleanup(stop);
  requestFrame();
  const mounted = (section: typeof sections[number]) => ready() >= sections.indexOf(section);
  const [levels, setLevels] = createSignal<5 | 10>(5);
  const [open, setOpen] = createSignal(false);
  createTrackedEffect(() => {
    props.symbol;
    setOpen(false);
    setTicket(null);
  });
  const revision = useMarketPulse(() => ticket() === null && !props.transitioning);
  const quote = createMemo(() => { revision(); return { ...props.engine.quote(props.symbol) }; });
  const trades = createMemo(() => {
    quote();
    const ring = props.engine.ring(props.symbol);
    const ticks: Tick[] = [];
    for (let index = ring.length - 1; index >= Math.max(0, ring.length - 14); index--) {
      const tick = ring.at(index);
      if (tick.side) ticks.push(tick);
    }
    return ticks;
  });
  const depth = createMemo(() => book(quote(), trades()[0]?.side, props.engine.ins(props.symbol).lot));
  const flow = createMemo(() => {
    quote();
    const { buy, sell } = props.engine.analytics(props.symbol);
    return { buy: buy.slice(), sell: sell.slice() };
  });
  const order = (side: OrderSide, price = quote().last) => {
    if (props.engine.ins(props.symbol).index) return;
    setTicket({ side, price });
    setOpen(true);
  };
  const activity = () => <>
    <Show when={mounted("book")}>
      <OrderBook book={depth()} count={levels()} previous={quote().prevClose} onToggle={() => setLevels((value) => value === 5 ? 10 : 5)} onLevel={order} />
      <Rule />
    </Show>
    <Show when={mounted("tape")}><Tape ticks={trades()} dayStart={quote().dayStart} /><Rule /></Show>
  </>;
  const ranges = () => <><Ranges bars={props.engine.bars(props.symbol)} quote={quote()} /><Rule /></>;
  const tradeable = () => !props.engine.ins(props.symbol).index;
  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <Row style={filling}>
      <ScrollArea style={filling}>
        <Show when={mounted("quote")}>
          <QuoteHeader engine={props.engine} quote={quote()} wide={!!props.wide} />
          <Rule />
        </Show>
        <Show when={mounted("chart")}>
          <StockChart engine={props.engine} symbol={props.symbol} revision={revision()} height={props.split ? 320 : props.wide ? 270 : 204} />
          <Rule />
        </Show>
        <Show when={props.split && mounted("ranges")}>{ranges()}</Show>
        <Show when={!props.split}>{activity()}</Show>
        <Show when={mounted("depth")}><Depth book={depth()} /><Rule /></Show>
        <Show when={mounted("flow")}><Flow data={flow()} /><Rule /></Show>
        <Show when={!props.split && mounted("ranges")}>{ranges()}</Show>
      </ScrollArea>
      <Show when={props.split}>
        <Rule vertical />
        <ScrollArea style={{ width: "39%", height: "100%", flex: 0, flexShrink: 0, minWidth: 290 }}>
          {activity()}
        </ScrollArea>
      </Show>
    </Row>
    <Rule />
    <Row style={{ backgroundColor: colors.bar, paddingLeft: 16, paddingRight: 16, paddingTop: 8, paddingBottom: 8,
      gap: 10, alignItems: "center", flexShrink: 0 }}>
      <Column style={{ width: 92, minWidth: 0, gap: 2 }}>
        <Label size={16} weight={600}>{fmtPrice(quote().last)}</Label>
        <Label size={12} color={tone(change(quote()))}>{`${sign(change(quote()))}%`}</Label>
      </Column>
      <Show when={tradeable()} fallback={<Label color={colors.dim}>{loc("指数不可交易", "Index · not tradable")}</Label>}>
        <Action label={loc("卖出", "Sell")} onClick={() => order("sell")} style={{ flex: 1, minHeight: 36 }} />
        <Action label={loc("买入", "Buy")} onClick={() => order("buy")} style={{ flex: 1, minHeight: 36 }} primary />
      </Show>
    </Row>
    <Show when={ticket()} keyed>{(seed) => <TicketOverlay engine={props.engine} symbol={props.symbol} seed={seed} open={open()} wide={!!props.wide}
      onClose={() => setOpen(false)} onClosed={() => { if (!open()) setTicket(null); }} />}</Show>
  </Column>;
}

export function StockPage(props: { router: Router; symbol: string; transitioning: boolean }) {
  const { engine } = useQuotes();
  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <Row style={{ minHeight: 48, paddingLeft: 8, paddingRight: 12, alignItems: "center", gap: 8 }}>
      <Action label={loc("‹ 返回", "‹ Back")} onClick={() => props.router.pop()} style={{ backgroundColor: "#00000000" }} />
      <Label size={18} weight={600} style={{ flex: 1 }}>{code(props.symbol)}</Label>
      <Favorite symbol={props.symbol} />
      <ClockButton />
    </Row>
    <Rule />
    <StockView engine={engine} symbol={props.symbol} transitioning={props.transitioning} />
  </Column>;
}
