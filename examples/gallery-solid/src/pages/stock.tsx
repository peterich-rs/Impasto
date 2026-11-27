import { For, Show, createMemo, createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { Canvas, Column, RepaintBoundary, Row, ScrollArea, View, useTheme } from "@impasto/solid-ui";
import { usePageActive, type Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { fmtPrice, fmtSigned } from "../format.ts";
import { marketOpen, minute, sectorOf } from "../engine/analytics.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { stockSnapshot, type StockSnapshot } from "../engine/stock-snapshot.ts";
import type { OrderSide } from "../engine/types.ts";
import { marketRunning, useMarketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { inWatchlist, toggleWatch } from "../state/watchlist.ts";
import { Flow, historicalRange, Ranges, Tape } from "../widgets/stocks/analysis.tsx";
import { dot, StockChart, stroke } from "../widgets/stocks/chart.tsx";
import { Depth, OrderBook } from "../widgets/stocks/order-book.tsx";
import { trackStockSections } from "../widgets/stocks/section-visibility.ts";
import { priceSlotWidth, Action, change, ClockButton, clock, code, colors, compact, filling, Label, Rule, Section, sectorLabel, sign, tone } from "../widgets/stocks/ui.tsx";
import { BackButton, Glyph } from "../widgets/stocks/icons.tsx";
import { TicketOverlay, type TicketSeed } from "./order.tsx";

declare const __STOCK_REPLAY_FREEZE__: "" | "book" | "chart";
const replayFreeze = typeof __STOCK_REPLAY_FREEZE__ === "undefined" ? "" : __STOCK_REPLAY_FREEZE__;

function QuoteHeader(props: { engine: QuoteEngine; data: StockSnapshot; time: string; wide: boolean }) {
  const theme = useTheme();
  const quote = () => props.data.quote;
  const price = createMemo(() => fmtPrice(quote().last));
  const priceSize = () => props.wide ? 38 : 36;
  const priceStyle = createMemo(() => {
    const em = priceSize() * theme().textScale;
    return {
      width: priceSlotWidth(priceSize(), theme().textScale),
      fontFamily: "Monospace",
      textAlign: "right" as const,
      height: Math.ceil(em * 1.2),
      flexShrink: 1,
    };
  });
  const instrument = () => props.engine.ins(quote().symbol);
  const color = () => tone(quote().last - quote().prevClose);
  const stats = [0, 1, 2, 3, 4, 5, 6, 7];
  const labels = () => [loc("今开", "Open"), loc("昨收", "Prev close"), loc("均价", "Avg price"), loc("振幅", "Amplitude"),
    loc("成交量", "Volume"), loc("成交额", "Turnover"), loc("120 日高", "120d high"), loc("120 日低", "120d low")];
  const values = createMemo(() => {
    const q = quote();
    const { turnover, history } = props.data;
    const [low, high] = historicalRange(history, q, 120);
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
      <Label size={11} color={colors.dim}>{props.time}</Label>
    </Row>
    <Row style={{ gap: 8, alignItems: "center", justifyContent: "space-between" }}>
      <Row style={{ gap: 6, alignItems: "center", minWidth: 0, flexShrink: 1 }}>
        <Label size={priceSize()} style={priceStyle()} weight={600} color={color()}>{price()}</Label>
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
  return <Glyph name={saved() ? "starFilled" : "star"} title={saved() ? loc("移出自选", "Remove from watchlist") : loc("加入自选", "Add to watchlist")}
    onClick={() => toggleWatch(props.symbol)} size={20} color={saved() ? colors.amber : colors.dim}
    style={{ backgroundColor: "#00000000" }} />;
}

export function StockView(props: { engine: QuoteEngine; symbol: string; wide?: boolean; split?: boolean; transitioning?: boolean }) {
  const sections = ["quote", "chart", "book", "tape", "depth", "flow", "ranges"] as const;
  const [ready, setReady] = createSignal(0);
  const active = usePageActive();
  const [ticket, setTicket] = createSignal<TicketSeed | null>(null);
  // Mount the quote and chart on entry. Resume staging when the slide or ticket closes.
  createTrackedEffect(() => {
    const next = ready() + 1;
    if (!active() || next === sections.length) return;
    if (next > sections.indexOf("chart") && (props.transitioning || ticket() !== null)) return;
    const stop = onFrame(() => {
      stop();
      setReady(next);
    });
    onCleanup(stop);
    requestFrame();
  });
  const mounted = (section: typeof sections[number]) => ready() >= sections.indexOf(section);
  const [levels, setLevels] = createSignal<5 | 10>(5);
  const [open, setOpen] = createSignal(false);
  createTrackedEffect(() => {
    props.symbol;
    setOpen(false);
    setTicket(null);
  });
  const revision = useMarketPulse(() => active() && ticket() === null && !props.transitioning);
  const snapshot = createMemo((previous?: StockSnapshot) => {
    revision();
    return stockSnapshot(props.engine, props.symbol, previous);
  });
  const quote = () => snapshot().quote;
  const time = createMemo(() => {
    revision();
    return clock(marketOpen * minute + props.engine.simNow() - quote().dayStart);
  });
  const tapeCount = createMemo(() => snapshot().trades.length);
  const main = trackStockSections();
  const side = trackStockSections();
  const chartSection = main.section();
  const rangeSection = main.section();
  const depthSection = main.section();
  const flowSection = main.section();
  const bookMain = main.section();
  const tapeMain = main.section();
  const bookSide = side.section();
  const tapeSide = side.section();
  const nearViewport = (visible: () => boolean, frozen = false) => createMemo((previous?: StockSnapshot) =>
    previous?.quote.symbol === props.symbol && (frozen || !visible()) ? previous : snapshot());
  const chart = nearViewport(chartSection.visible, replayFreeze === "chart");
  const range = nearViewport(rangeSection.visible);
  const depth = nearViewport(depthSection.visible);
  const flow = nearViewport(flowSection.visible);
  const orderBook = nearViewport(() => props.split ? bookSide.visible() : bookMain.visible(), replayFreeze === "book");
  const tape = nearViewport(() => props.split ? tapeSide.visible() : tapeMain.visible());
  const order = (side: OrderSide, price = quote().last) => {
    if (props.engine.ins(props.symbol).index) return;
    setTicket({ side, price });
    setOpen(true);
  };
  const activity = (bookSection: typeof bookMain, tapeSection: typeof tapeMain) => <>
    <Show when={mounted("book")}>
      <Column ref={bookSection.ref}>
        <OrderBook book={orderBook().book} count={levels()} previous={orderBook().quote.prevClose} onToggle={() => setLevels((value) => value === 5 ? 10 : 5)} onLevel={order} />
        <Rule />
      </Column>
    </Show>
    <Show when={mounted("tape")}>
      <Column ref={tapeSection.ref}>
        <Tape ticks={tape().trades} dayStart={tape().quote.dayStart} rows={tapeCount()} />
        <Rule />
      </Column>
    </Show>
  </>;
  const ranges = () => <Column ref={rangeSection.ref}><Ranges bars={range().history} quote={range().quote} /><Rule /></Column>;
  const tradeable = () => !props.engine.ins(props.symbol).index;
  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <Row style={filling}>
      <ScrollArea style={filling} controller={main.controller}>
        <Show when={mounted("quote")}>
          <QuoteHeader engine={props.engine} data={snapshot()} time={time()} wide={!!props.wide} />
          <Rule />
        </Show>
        <Show when={mounted("chart")}>
          <Column ref={chartSection.ref}>
            <StockChart snapshot={chart()} height={props.split ? 320 : props.wide ? 270 : 204} />
            <Rule />
          </Column>
        </Show>
        <Show when={props.split && mounted("ranges")}>{ranges()}</Show>
        <Show when={!props.split}>{activity(bookMain, tapeMain)}</Show>
        <Show when={mounted("depth")}><Column ref={depthSection.ref}><Depth book={depth().book} /><Rule /></Column></Show>
        <Show when={mounted("flow")}><Column ref={flowSection.ref}><Flow data={flow().flow} /><Rule /></Column></Show>
        <Show when={!props.split && mounted("ranges")}>{ranges()}</Show>
      </ScrollArea>
      <Show when={props.split}>
        <Rule vertical />
        <ScrollArea style={{ width: "39%", height: "100%", flex: 0, flexShrink: 0, minWidth: 290 }} controller={side.controller}>
          {activity(bookSide, tapeSide)}
        </ScrollArea>
      </Show>
    </Row>
    <Rule />
    <Row style={{ backgroundColor: colors.bar, paddingLeft: 16, paddingRight: 16, paddingTop: 8, paddingBottom: 8,
      gap: 10, alignItems: "center", flexShrink: 0 }}>
      <RepaintBoundary style={{ flexDirection: "column", width: 92, minWidth: 0, gap: 2 }}>
        <Label size={16} weight={600} style={{ fontFamily: "Monospace", textAlign: "right" }}>{fmtPrice(quote().last)}</Label>
        <Label size={12} color={tone(change(quote()))}>{`${sign(change(quote()))}%`}</Label>
      </RepaintBoundary>
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
    <Row style={{ minHeight: 52, paddingLeft: 4, paddingRight: 8, alignItems: "center", gap: 4 }}>
      <BackButton onClick={() => props.router.pop()} />
      <Label size={18} weight={600} style={{ flex: 1 }}>{code(props.symbol)}</Label>
      <Favorite symbol={props.symbol} />
      <ClockButton />
    </Row>
    <Rule />
    <StockView engine={engine} symbol={props.symbol} transitioning={props.transitioning} />
  </Column>;
}
