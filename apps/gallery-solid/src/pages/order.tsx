import { createSignal, createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { Button, Dialog, Sheet } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { accountRef } from "../state/account-ref.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import type { Router } from "@impasto/solid-router";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const UP = colorBytes("#EB3E3E");
const DOWN = colorBytes("#22B05F");

/** Order ticket sheet: side toggle, price, qty stepper, percent quick bars. */
export function OrderSheet(props: {
  router: Router;
  symbol: string;
  query: Record<string, string>;
  onClose: () => void;
}): NodeHandle {
  const { engine } = useQuotes();
  const account = accountRef();
  const symbol = () => props.symbol;
  const q = () => engine.quote(symbol());
  const [side, setSide] = createSignal<"buy" | "sell">(
    props.query.side === "sell" ? "sell" : "buy",
  );
  const [type, setType] = createSignal<"market" | "limit">("limit");
  const [price, setPrice] = createSignal(
    props.query.price ? Number(props.query.price) : q().last,
  );
  const [qty, setQty] = createSignal(100);
  const [confirming, setConfirming] = createSignal(false);
  const [error, setError] = createSignal("");

  const ins = engine.ins(symbol());
  const pageInner = host.column(8);
  const page = host.padding(0);
  host.setPaddingEdges(page, 16, 16, 16, 16);
  host.append(page, pageInner);

  // title
  const title = host.text(`${ins.name} 下单`, 17, INK);
  host.setFontWeight(title, 700);
  host.append(pageInner, title);

  // side toggle
  const sideRow = host.row(0);
  host.append(pageInner, sideRow);
  for (const s of ["buy", "sell"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setSide(s));
    host.append(sideRow, cell);
    const pill = host.decorated(PANEL, 8);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 36);
    host.append(cell, pill);
    const label = host.text(s === "buy" ? "买入" : "卖出", 14, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(pill, align);
    createTrackedEffect(() => {
      const active = side() === s;
      host.setColor(pill, active ? (s === "buy" ? UP : DOWN) : PANEL);
      host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
    });
  }

  // order type
  const typeRow = host.row(0);
  host.append(pageInner, typeRow);
  for (const t of ["market", "limit"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setType(t));
    host.append(typeRow, cell);
    const label = host.text(t === "market" ? "市价" : "限价", 13, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(cell, align);
    createTrackedEffect(() => {
      const active = type() === t;
      host.setColor(label, active ? INK : MUTED);
      host.setFontWeight(label, active ? 600 : 400);
    });
  }

  // price row (limit only)
  const priceBox = host.column(4);
  host.append(pageInner, priceBox);
  const priceLabel = host.text("价格", 12, MUTED);
  host.append(priceBox, priceLabel);
  const priceRow = host.row(6);
  host.append(priceBox, priceRow);
  const priceDown = stepperBtn("−", () => setPrice((p) => Math.max(0.01, round2(p - tick()))));
  host.append(priceRow, priceDown);
  const priceValue = host.text(fmtPrice(price()), 16, INK);
  host.setFontWeight(priceValue, 600);
  host.setGrow(priceValue, 0);
  const priceGrow = host.sized(0, 0);
  host.setGrow(priceGrow, 1);
  host.append(priceRow, priceValue);
  host.append(priceRow, priceGrow);
  const priceUp = stepperBtn("+", () => setPrice((p) => round2(p + tick())));
  host.append(priceRow, priceUp);
  createTrackedEffect(() => {
    host.setText(priceValue, type() === "market" ? "市价成交" : fmtPrice(price()));
  });
  createTrackedEffect(() => {
    const hide = type() === "market";
    host.setAlpha(priceDown, hide ? 80 : 255);
    host.setAlpha(priceUp, hide ? 80 : 255);
  });

  function tick(): number {
    return ins.market === "cn" ? 0.01 : q().last > 100 ? 0.05 : 0.01;
  }

  // qty stepper
  const qtyBox = host.column(4);
  host.append(pageInner, qtyBox);
  const qtyLabel = host.text(
    `数量（${ins.lot === 100 ? "按手，1手=100股" : "股数"}）`,
    12,
    MUTED,
  );
  host.append(qtyBox, qtyLabel);
  const qtyRow = host.row(6);
  host.append(qtyBox, qtyRow);
  const qtyDown = stepperBtn("−", () => setQty((n) => Math.max(ins.lot, n - ins.lot)));
  host.append(qtyRow, qtyDown);
  const qtyValue = host.text(fmtQty(qty()), 16, INK);
  host.setFontWeight(qtyValue, 600);
  host.append(qtyRow, qtyValue);
  const qtyGrow = host.sized(0, 0);
  host.setGrow(qtyGrow, 1);
  host.append(qtyRow, qtyGrow);
  const qtyUp = stepperBtn("+", () => setQty((n) => n + ins.lot));
  host.append(qtyRow, qtyUp);
  createTrackedEffect(() => {
    host.setText(qtyValue, fmtQty(qty()));
  });

  // percent quick bars (based on cash or holding)
  const pctRow = host.row(6);
  host.append(pageInner, pctRow);
  for (const pct of [25, 50, 75, 100]) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => {
      if (side() === "buy") {
        const spendable = account.cash() / (1 + 0.0003);
        const lots = Math.floor((spendable * pct) / 100 / (price() * ins.lot)) * ins.lot;
        setQty(Math.max(ins.lot, lots));
      } else {
        const held = account.holding(symbol());
        const lots = Math.floor((held * pct) / 100 / ins.lot) * ins.lot;
        setQty(Math.max(ins.lot, lots));
      }
    });
    host.append(pctRow, cell);
    const pill = host.decorated(colorBytes("#F2F3F5"), 6);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 26);
    host.append(cell, pill);
    const label = host.text(`${pct}%`, 11, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(pill, align);
  }

  // estimated amount
  const amount = host.text("", 12, MUTED);
  host.append(pageInner, amount);
  createTrackedEffect(() => {
    const eff = type() === "market" ? q().last : price();
    const total = eff * qty();
    host.setText(amount, `约 ${fmtPrice(total)} · 手续费 ${fmtPrice(account.feeOf(total))} · 可用 ${fmtPrice(account.cash())}`);
  });

  // error line
  const errText = host.text("", 12, UP);
  host.append(pageInner, errText);
  createTrackedEffect(() => {
    host.setText(errText, error());
  });

  // submit
  const submit = host.pointerRegion();
  host.setGrow(submit, 1);
  host.onClick(submit, () => {
    const eff = type() === "market" ? q().last : price();
    const result = account.submit(symbol(), side(), type(), eff, qty());
    if (!result.ok) {
      setError(result.reason);
      return;
    }
    setError("");
    setConfirming(true);
  });
  const submitPill = host.decorated(UP, 999);
  host.setMinSize(submitPill, 0, 44);
  host.append(submit, submitPill);
  const submitLabel = host.text("提交订单", 15, colorBytes("#FFFFFF"));
  host.setFontWeight(submitLabel, 600);
  const submitAlign = host.align("center");
  host.append(submitAlign, submitLabel);
  host.append(submitPill, submitAlign);
  createTrackedEffect(() => {
    host.setColor(submitPill, side() === "buy" ? UP : DOWN);
  });
  host.append(pageInner, submit);

  function stepperBtn(glyph: string, onClick: () => void): number {
    const cell = host.pointerRegion();
    host.onClick(cell, onClick);
    const box = host.decorated(colorBytes("#F2F3F5"), 8);
    host.setMinSize(box, 36, 36);
    host.append(cell, box);
    const label = host.text(glyph, 18, INK);
    const align = host.align("center");
    host.append(align, label);
    host.append(box, align);
    return cell;
  }

  return {
    id: page,
    open: () => setConfirming(true),
    closeConfirm: () => setConfirming(false),
  };
}

function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

export function OrderSheetHost(props: {
  router: Router;
  symbol: string;
  query: Record<string, string>;
}): NodeHandle {
  const [open, setOpen] = createSignal(true);
  return Sheet({
    open: open(),
    onClose: () => {
      setOpen(false);
      props.router.pop();
    },
    title: "",
    children: [],
  });
}

export { Dialog, Button };
