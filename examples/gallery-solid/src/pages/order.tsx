import { createSignal, For, Show, onCleanup, untrack } from "solid-js";
import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { Column, Dialog, Row, Sheet, View } from "@impasto/solid-ui";
import type { Router } from "@impasto/solid-router";
import { loc } from "../copy.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import type { OrderSide, OrderType } from "../engine/types.ts";
import { marketPulse, useQuotes } from "../state/quotes-provider.tsx";
import { accountRef } from "../state/account-ref.ts";
import { showToast } from "../state/toast.ts";
import { Action, code, colors, Label, Rule, Segments } from "../widgets/stocks/ui.tsx";
import { Glyph } from "../widgets/stocks/icons.tsx";

export type TicketSeed = { side: OrderSide; price: number };
const sides = ["buy", "sell"] as const;
const types = ["limit", "market"] as const;
const sizes = [100, 500, 1000, 5000];
const sideLabel = (side: OrderSide) => side === "buy" ? loc("买入", "Buy") : loc("卖出", "Sell");
const typeLabel = (type: OrderType) => type === "limit" ? loc("限价", "Limit") : loc("市价", "Market");

function OrderTicket(props: { engine: QuoteEngine; symbol: string; seed: TicketSeed; onClose: () => void }) {
  const [ready, setReady] = createSignal(0);
  let stage = 0;
  const stop = onFrame(() => {
    setReady(++stage);
    if (stage === 4) stop();
    else requestFrame();
  });
  onCleanup(stop);
  requestFrame();
  const account = accountRef();
  // A mounted ticket keeps its instrument and seeds the editable draft once.
  const { engine, symbol, seed } = untrack(() => ({
    engine: props.engine,
    symbol: props.symbol,
    seed: props.seed,
  }));
  const instrument = engine.ins(symbol);
  const [side, setSide] = createSignal(seed.side);
  const [type, setType] = createSignal<OrderType>("limit");
  const [cents, setCents] = createSignal(Math.round(seed.price * 100));
  const lotSize = (size: number) => Math.ceil(size / instrument.lot) * instrument.lot;
  const quantities = [...new Set(sizes.map(lotSize))];
  const [qty, setQty] = createSignal(lotSize(500));
  const [review, setReview] = createSignal(false);
  const [reviewPrice, setReviewPrice] = createSignal(0);
  const [error, setError] = createSignal("");
  const last = () => { marketPulse(); return engine.quote(symbol).last; };
  const price = () => type() === "market" ? last() : cents() / 100;
  const amount = () => (review() ? reviewPrice() : price()) * qty();
  const fee = () => account.feeOf(amount());
  const balance = () => side() === "buy" ? account.availableCash() : account.availableHolding(symbol);
  const percent = () => side() === "buy" ? (amount() + fee()) / Math.max(balance(), 1) : qty() / Math.max(balance(), 1);
  const submit = () => {
    const quote = price();
    const result = account.submit(symbol, side(), type(), quote, qty());
    if (!result.ok) {
      setError(result.reason);
      return;
    }
    showToast(loc(`${sideLabel(side())} ${code(symbol)} · 已提交模拟订单`, `${sideLabel(side())} ${code(symbol)} · Paper order placed`));
    props.onClose();
  };

  return <Column style={{ gap: 10, padding: 16, minWidth: 0 }}>
    <Row style={{ alignItems: "center", gap: 8 }}>
      <Label size={20} weight={600} style={{ flex: 1 }}>{`${sideLabel(side())} ${code(symbol)}`}</Label>
      <Label size={11} color={colors.faint}>{loc("模拟下单", "Paper order")}</Label>
      <Glyph name="close" title={loc("关闭", "Close")} onClick={props.onClose} size={18} color={colors.dim} style={{ backgroundColor: "#00000000" }} />
    </Row>
    <Show when={!review()} fallback={<Column style={{ gap: 8 }}>
      <Rule />
      <Label size={14} weight={600}>{loc("复核订单", "Review order")}</Label>
      <Label color={colors.dim}>{`${instrument.name} · ${typeLabel(type())}`}</Label>
      <Label>{`${fmtQty(qty())} × ${fmtPrice(reviewPrice())}`}</Label>
    </Column>}>
      <Show when={ready() >= 1}><Row style={{ gap: 8, flexWrap: "wrap" }}>
        <Segments options={sides} selected={side()} label={sideLabel} onSelect={(value) => { setSide(value); setError(""); }} />
        <Segments options={types} selected={type()} label={typeLabel} onSelect={(value) => { setType(value); setError(""); }} />
      </Row></Show>
      <Show when={ready() >= 2}><Row style={{ justifyContent: "space-between", alignItems: "center", gap: 6 }}>
        <Label color={colors.dim}>{type() === "limit" ? loc("限价", "Limit price") : loc("市价", "Market price")}</Label>
        <Show when={type() === "limit"} fallback={<Label size={18} weight={600}>{fmtPrice(last())}</Label>}>
          <Action label={loc(`用现价 ${fmtPrice(last())}`, `Use last ${fmtPrice(last())}`)} onClick={() => setCents(Math.round(last() * 100))} />
        </Show>
      </Row>
      <Show when={type() === "limit"}>
        <Row style={{ alignItems: "center", gap: 8 }}>
          <Glyph name="minus" title={loc("降低价格", "Lower price")} onClick={() => setCents((value) => Math.max(1, value - 1))} style={{ width: 40, height: 40, borderRadius: 8 }} />
          <View style={{ flex: 1, minHeight: 38, justifyContent: "center", alignItems: "center", borderWidth: 1, borderColor: colors.line, borderRadius: 7 }}>
            <Label size={18} weight={600}>{fmtPrice(price())}</Label>
          </View>
          <Glyph name="plus" title={loc("提高价格", "Raise price")} onClick={() => setCents((value) => value + 1)} style={{ width: 40, height: 40, borderRadius: 8 }} />
        </Row>
      </Show>
      </Show>
      <Show when={ready() >= 3}><Row style={{ justifyContent: "space-between" }}>
        <Label color={colors.dim}>{loc("数量", "Quantity")}</Label>
        <Label color={colors.dim}>{loc(`${fmtQty(qty())} 股`, `${fmtQty(qty())} sh`)}</Label>
      </Row>
      <Row style={{ gap: 8 }}>
        <For each={quantities}>{(size) => <Action label={size >= 1000 ? `${size / 1000}K` : String(size)}
          primary={qty() === size} onClick={() => setQty(size)} style={{ flex: 1, minHeight: 36, padding: 0 }} />}</For>
      </Row></Show>
    </Show>
    <Show when={ready() >= 4}><Row style={{ justifyContent: "space-between", alignItems: "baseline", gap: 8 }}>
      <Label color={colors.dim}>{side() === "buy" ? loc("预估成本", "Estimated cost") : loc("预估收入", "Estimated proceeds")}</Label>
      <Label size={19} weight={600}>{fmtPrice(side() === "buy" ? amount() + fee() : amount() - fee())}</Label>
    </Row>
    <View style={{ height: 5, backgroundColor: colors.raised, borderRadius: 3, overflow: "hidden" }}>
      <View style={{ height: 5, width: `${Math.min(1, percent()) * 100}%`, backgroundColor: percent() > 1 ? colors.down : colors.lime }} />
    </View>
    <Row style={{ gap: 6, justifyContent: "space-between", flexWrap: "wrap" }}>
      <Label size={11} color={colors.faint}>{loc(`预估费用 ${fmtPrice(fee())}`, `Est. fee ${fmtPrice(fee())}`)}</Label>
      <Label size={11} color={colors.faint}>{side() === "buy" ? loc(`可用 ${fmtPrice(balance())}`, `Available ${fmtPrice(balance())}`) : loc(`可卖 ${fmtQty(balance())} 股`, `Available ${fmtQty(balance())} sh`)}</Label>
    </Row>
    <Show when={review() && type() === "market"}>
      <Label size={11} color={colors.dim} style={{ maxLines: 2 }}>{loc("市价单按提交时的最新模拟价成交", "Market orders fill at the simulated price when submitted.")}</Label>
    </Show>
    <Show when={error()}><Label color={colors.down} style={{ maxLines: 3 }}>{error()}</Label></Show>
    <Row style={{ gap: 8 }}>
      <Show when={review()}><Action label={loc("修改", "Edit")} onClick={() => { setReview(false); setError(""); }} style={{ minHeight: 40 }} /></Show>
      <Action label={review() ? loc("确认订单", "Confirm order") : loc("复核订单", "Review order")} primary onClick={() => {
        if (review()) submit();
        else { setReviewPrice(price()); setReview(true); setError(""); }
      }} style={{ flex: 1, minHeight: 40 }} />
    </Row></Show>
  </Column>;
}

export function TicketOverlay(props: { engine: QuoteEngine; symbol: string; seed: TicketSeed; open: boolean; wide: boolean; onClose: () => void; onClosed?: () => void }) {
  const content = () => <OrderTicket engine={props.engine} symbol={props.symbol} seed={props.seed} onClose={props.onClose} />;
  return <Show when={props.wide} fallback={<Sheet open={props.open} onClose={props.onClose} onClosed={props.onClosed}
    style={{ width: "100%", backgroundColor: colors.bar, padding: 0 }}>{content()}</Sheet>}>
    <Dialog open={props.open} onClose={props.onClose} onClosed={props.onClosed} style={{ width: 400, maxWidth: "92%", backgroundColor: colors.bar, padding: 0 }}>{content()}</Dialog>
  </Show>;
}

export function OrderSheetHost(props: { router: Router; symbol: string }) {
  const { engine } = useQuotes();
  return <TicketOverlay engine={engine} symbol={props.symbol} seed={{ side: "buy", price: engine.quote(props.symbol).last }}
    open wide={false} onClose={() => props.router.pop()} />;
}
