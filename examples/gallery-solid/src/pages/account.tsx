import { createMemo, createSignal, For, Show } from "solid-js";
import type { NodeHandle } from "@impasto/solid-renderer";
import { Column, Row, View, Pressable, Role, Dialog, ScrollArea } from "@impasto/solid-ui";
import type { QuoteEngine } from "../engine/engine.ts";
import { usePageActive, type Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import type { Position, Order } from "../engine/types.ts";
import { accountRef } from "../state/account-ref.ts";
import { marketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { showToast } from "../state/toast.ts";
import { Action, change, code, colors, filling, Label, PriceTag, Rule, Section, Segments, sign, tone } from "../widgets/stocks/ui.tsx";

type Book = "positions" | "open" | "fills";
const books: readonly Book[] = ["positions", "open", "fills"];
const bookLabel = (book: Book) => book === "positions" ? loc("持仓", "Positions") : book === "open" ? loc("待成交", "Open") : loc("已成交", "Fills");

type Row = {
  position: Position;
  qty: number;
  cost: number;
  last: number;
  prevClose: number;
  marketValue: number;
  weight: number;
  dayPnl: number;
  dayPct: number;
  totalPnl: number;
  totalPct: number;
};

export function AccountPage(props: { router: Router }): NodeHandle {
  const { engine } = useQuotes();
  const account = accountRef();
  const active = usePageActive();
  const [book, setBook] = createSignal<Book>("positions");
  const [closing, setClosing] = createSignal<string | null>(null);

  const rows = createMemo<Row[]>((previous = []) => {
    if (!active()) return previous;
    marketPulse();
    const positions = account.positions();
    let marketValue = 0;
    const priced = positions.map((position) => {
      const quote = engine.quote(position.symbol);
      const value = position.qty * quote.last;
      marketValue += value;
      return { position, quote, value };
    });
    const equity = account.cash() + marketValue;
    return priced.map(({ position, quote, value }) => ({
      position,
      qty: position.qty,
      cost: position.avgCost,
      last: quote.last,
      prevClose: quote.prevClose,
      marketValue: value,
      weight: equity > 0 ? value / equity * 100 : 0,
      dayPnl: position.qty * (quote.last - quote.prevClose),
      dayPct: change(quote),
      totalPnl: position.qty * (quote.last - position.avgCost),
      totalPct: position.avgCost > 0 ? (quote.last - position.avgCost) / position.avgCost * 100 : 0,
    })).sort((a, b) => b.marketValue - a.marketValue);
  });
  const totals = createMemo(() => {
    const list = rows();
    const marketValue = list.reduce((sum, row) => sum + row.marketValue, 0);
    const cost = list.reduce((sum, row) => sum + row.qty * row.cost, 0);
    const dayPnl = list.reduce((sum, row) => sum + row.dayPnl, 0);
    const cash = account.cash();
    const equity = cash + marketValue;
    const openCount = account.orders().filter((order) => order.status === "pending").length;
    return { cash, marketValue, cost, dayPnl, equity, totalPnl: marketValue - cost, openCount,
      dayPct: equity - dayPnl > 0 ? dayPnl / (equity - dayPnl) * 100 : 0 };
  });

  return <Column style={[filling, { backgroundColor: colors.bg }]}>
    <ScrollArea style={filling}>
    <Section title={loc("账户", "Account")} note={loc("模拟盘 · 数据为虚构", "Paper · fictional data")}>
      <Column style={{ gap: 6 }}>
        <Label size={12} color={colors.dim}>{loc("总资产", "Equity")}</Label>
        <Label size={32} weight={600}>{fmtPrice(totals().equity)}</Label>
        <Row style={{ gap: 8, alignItems: "baseline" }}>
          <Label size={12} color={colors.dim}>{loc("今日盈亏", "Today")}</Label>
          <Label size={14} weight={600} color={tone(totals().dayPct)}>{fmtSignedPnl(totals().dayPnl)}</Label>
          <Label size={12} color={tone(totals().dayPct)}>{`${sign(totals().dayPct)}%`}</Label>
        </Row>
      </Column>
      <View style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0px, 1fr))", columnGap: 8 }}>
        <Stat label={loc("可用资金", "Cash")} value={fmtPrice(totals().cash)} />
        <Stat label={loc("持仓市值", "Market value")} value={fmtPrice(totals().marketValue)} />
        <Stat label={loc("累计盈亏", "Total P&L")} value={fmtSignedPnl(totals().totalPnl)} tone={tone(totals().totalPnl)} />
      </View>
    </Section>
    <Rule />
    <Section>
      <Segments options={books} selected={book()} label={(item) => {
        const count = item === "positions" ? rows().length : item === "open" ? totals().openCount : undefined;
        return `${bookLabel(item)}${count !== undefined ? ` ${count}` : ""}`;
      }} onSelect={setBook} />
    </Section>
    <Show when={book() === "positions"} fallback={<Orders book={book() === "fills" ? "fills" : "open"} orders={account.orders()} engine={engine} onCancel={(id) => { account.cancel(id); showToast(loc("已撤单", "Cancelled")); }} onOpen={(symbol) => props.router.push(`/stock/${symbol}`)} />}>
      <Show when={rows().length === 0} fallback={
        <For each={rows()} keyed={(row) => row.position.symbol}>
          {(row) => <PositionRow row={row()} name={(symbol) => engine.ins(symbol).name} onClick={() => props.router.push(`/stock/${row().position.symbol}`)} />}
        </For>
      }>
        <Column style={{ padding: 24, gap: 12, alignItems: "center" }}>
          <Label color={colors.dim}>{loc("暂无持仓，去自选页买入", "No positions yet. Buy from the watchlist")}</Label>
          <Action label={loc("去自选", "Watchlist")} onClick={() => props.router.reset("/watchlist")} primary />
        </Column>
      </Show>
    </Show>
    {/* Clears the floating route capsule. */}
    <View style={{ height: 72, flexShrink: 0 }} />
    </ScrollArea>
    <Dialog open={closing() !== null} onClose={() => setClosing(null)} title={loc("平仓", "Close position")}>
      <Column style={{ gap: 12 }}>
        <Label size={14} style={{ maxLines: 3 }}>{closing() ? loc(`确认平掉 ${engine.ins(closing()!).name} 全部持仓？`, `Close all ${engine.ins(closing()!).name}?`) : ""}</Label>
        <Row style={{ gap: 8 }}>
          <Action label={loc("取消", "Cancel")} onClick={() => setClosing(null)} style={{ flex: 1 }} />
          <Action label={loc("确认平仓", "Close")} primary onClick={() => {
            const symbol = closing();
            setClosing(null);
            if (!symbol) return;
            const result = account.flatten(symbol);
            showToast(result.ok ? loc("已平仓", "Closed") : result.reason);
          }} style={{ flex: 1 }} />
        </Row>
      </Column>
    </Dialog>
  </Column>;
}

function Stat(props: { label: string; value: string; tone?: string }) {
  return <Column style={{ gap: 4, minWidth: 0 }}>
    <Label size={11} color={colors.faint}>{props.label}</Label>
    <Label size={14} weight={600} color={props.tone ?? colors.ink}>{props.value}</Label>
  </Column>;
}

/**
 * Two-band position row. Upper band: identity, size, price badge. Lower band:
 * cost, weight and today's P&L in three equal columns so every value stays on
 * the same grid line across rows.
 */
function PositionRow(props: { row: Row; name: (symbol: string) => string; onClick: () => void }) {
  const row = () => props.row;
  return <Column style={{ gap: 0 }}>
    <Pressable role={Role.Button} onClick={props.onClick} accessibilityLabel={`${code(row().position.symbol)} ${fmtQty(row().qty)}`}
      style={{ paddingLeft: 16, paddingRight: 16, paddingTop: 12, paddingBottom: 12, flexDirection: "column", gap: 10 }}>
      <Row style={{ gap: 8, alignItems: "center", minWidth: 0 }}>
        <Column style={{ flex: 1, minWidth: 0, gap: 3 }}>
          <Row style={{ gap: 6, alignItems: "baseline", minWidth: 0 }}>
            <Label size={15} weight={600}>{code(row().position.symbol)}</Label>
            <Label size={12} color={colors.dim} style={{ flex: 1 }}>{`${props.name(row().position.symbol)} · ${loc(`${fmtQty(row().qty)}股`, `${fmtQty(row().qty)} sh`)}`}</Label>
          </Row>
          <Label size={11} color={colors.faint}>{loc(`成本 ${fmtPrice(row().cost)}`, `cost ${fmtPrice(row().cost)}`)}</Label>
        </Column>
        <PriceTag value={fmtPrice(row().last)} change={row().dayPct} />
      </Row>
      <View style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0px, 1fr))", columnGap: 8 }}>
        <Stat label={loc("成本价", "Cost")} value={fmtPrice(row().cost)} />
        <Stat label={loc("仓位", "Weight")} value={`${row().weight.toFixed(1)}%`} />
        <Column style={{ gap: 4, minWidth: 0, alignItems: "flex-end" }}>
          <Label size={11} color={colors.faint}>{loc("今日盈亏", "Today")}</Label>
          <Row style={{ gap: 6, alignItems: "baseline", justifyContent: "flex-end" }}>
            <Label size={13} weight={600} color={tone(row().dayPct)}>{fmtSignedPnl(row().dayPnl)}</Label>
            <Label size={12} color={tone(row().dayPct)}>{`${sign(row().dayPct)}%`}</Label>
          </Row>
        </Column>
      </View>
    </Pressable>
    <Rule />
  </Column>;
}

function Orders(props: { book: "open" | "fills"; orders: readonly Order[]; engine: QuoteEngine; onCancel: (id: string) => void; onOpen: (symbol: string) => void }) {
  const list = createMemo(() => props.orders.filter((order) => props.book === "open" ? order.status === "pending" : order.status === "filled"));
  return <Show when={list().length > 0} fallback={<Label color={colors.dim} style={{ padding: 24, textAlign: "center" }}>
    {props.book === "open" ? loc("没有待成交委托", "No open orders") : loc("暂无已成交", "No fills yet")}
  </Label>}>
    <For each={list()}>{(order) => <Column style={{ gap: 0 }}>
      <Pressable role={Role.Button} onClick={() => props.onOpen(order.symbol)} style={{ paddingLeft: 16, paddingRight: 16, paddingTop: 12, paddingBottom: 12, flexDirection: "row", gap: 10, alignItems: "center" }}>
        <View style={{ width: 4, height: 36, borderRadius: 2, backgroundColor: order.side === "buy" ? tone(1) : tone(-1) }} />
        <Column style={{ flex: 1, minWidth: 0, gap: 3 }}>
          <Row style={{ gap: 6, alignItems: "baseline" }}>
            <Label size={14} weight={600}>{code(order.symbol)}</Label>
            <Label size={12} color={colors.dim}>{props.engine.ins(order.symbol).name}</Label>
          </Row>
          <Label size={11} color={colors.faint}>{`${order.type === "market" ? loc("市价", "Mkt") : loc("限价", "Lmt")} ${fmtPrice(order.price)} · ${fmtQty(order.qty)}${loc("股", " sh")} · ${order.status === "pending" ? loc("待成交", "open") : loc("已成交", "filled")}`}</Label>
        </Column>
        <Show when={order.status === "pending"}><Action label={loc("撤单", "Cancel")} onClick={() => props.onCancel(order.id)} style={{ minHeight: 30 }} /></Show>
      </Pressable>
      <Rule />
    </Column>}</For>
  </Show>;
}

function fmtSignedPnl(value: number): string {
  return `${value >= 0 ? "+" : "-"}${fmtPrice(Math.abs(value))}`;
}
