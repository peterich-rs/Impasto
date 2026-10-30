import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { host, onFrame, type NodeHandle } from "@impasto/solid-renderer";
import { Button } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { clearChildren } from "../widgets/palette.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { accountRef } from "../state/account-ref.ts";
import { settingsSig } from "../state/settings.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import type { Router } from "@impasto/solid-router";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { QuoteText } from "../widgets/quote-text.tsx";
import { Monogram } from "../widgets/monogram.tsx";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const PRIMARY = colorBytes("#C8102E");

type Tab = "positions" | "pending" | "done";

export function TradePage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const account = accountRef();
  const [tab, setTab] = createSignal<Tab>("positions");

  const page = host.column(8);
  host.setGrow(page, 1);

  // === equity card ===
  const card = host.decorated(colorBytes("#171B26"), 16);
  const cardInner = host.padding(0);
  host.setPaddingEdges(cardInner, 16, 16, 16, 16);
  host.append(card, cardInner);
  const cardPad = host.padding(0);
  host.setPaddingEdges(cardPad, 12, 16, 0, 16);
  host.append(cardPad, card);
  host.append(page, cardPad);

  const cardTitle = host.text("总资产 (模拟)", 12, colorBytes("#9AA0A6"));
  host.append(cardInner, cardTitle);
  const equityRow = host.row(8);
  host.append(cardInner, equityRow);
  const equity = host.text("—", 30, colorBytes("#FFFFFF"));
  host.setFontWeight(equity, 700);
  host.append(equityRow, equity);
  const dayDelta = host.text("", 13, colorBytes("#9AA0A6"));
  host.append(equityRow, dayDelta);

  const statRow = host.row(0);
  host.append(cardInner, statRow);
  const cashStat = cardStat("可用资金");
  const mvStat = cardStat("持仓市值");
  const pnlStat = cardStat("浮动盈亏");
  for (const s of [cashStat, mvStat, pnlStat]) {
    host.setGrow(s.root, 1);
    host.append(statRow, s.root);
  }

  // refresh equity at 2Hz off the frame clock (not the per-tick path)
  let lastPaint = -1;
  onCleanup(
    onFrame((targetMs) => {
      if (lastPaint >= 0 && targetMs - lastPaint < 500) {
        return;
      }
      lastPaint = targetMs;
      const mv = account.marketValue();
      const total = account.cash() + mv;
      const pnl = mv - account.positions().reduce((s, p) => s + p.avgCost * p.qty, 0);
      host.setText(equity, fmtPrice(total));
      host.setText(cashStat.value, fmtPrice(account.cash()));
      host.setText(mvStat.value, fmtPrice(mv));
      host.setText(pnlStat.value, fmtPrice(pnl));
      host.setColor(pnlStat.value, pnl > 0 ? colorBytes("#FF7B72") : pnl < 0 ? colorBytes("#3FB950") : colorBytes("#9AA0A6"));
    }),
  );

  function cardStat(labelText: string): { root: number; value: number } {
    const col = host.column(2);
    const cap = host.text(labelText, 11, colorBytes("#9AA0A6"));
    host.append(col, cap);
    const value = host.text("—", 14, colorBytes("#FFFFFF"));
    host.setFontWeight(value, 600);
    host.append(col, value);
    return { root: col, value };
  }

  // === tabs ===
  const tabs = host.row(0);
  const tabPad = host.padding(0);
  host.setPaddingEdges(tabPad, 12, 16, 0, 16);
  host.append(tabPad, tabs);
  host.append(page, tabPad);
  for (const t of ["positions", "pending", "done"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setTab(t));
    host.append(tabs, cell);
    const inner = host.padding(0);
    host.setPaddingEdges(inner, 6, 0, 6, 0);
    host.append(cell, inner);
    const label = host.text(tabLabel(t), 13, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(inner, align);
    createTrackedEffect(() => {
      const active = tab() === t;
      host.setColor(label, active ? INK : MUTED);
      host.setFontWeight(label, active ? 700 : 400);
    });
  }

  // === list panel ===
  const panel = host.decorated(PANEL, 12);
  host.setStroke(panel, 1, HAIR);
  host.setGrow(panel, 1);
  const panelPad = host.padding(0);
  host.setPaddingEdges(panelPad, 12, 16, 16, 16);
  host.append(panelPad, panel);
  host.append(page, panelPad);

  const list = host.column(0);
  host.append(panel, list);

  createTrackedEffect(() => {
    clearChildren(list);
    if (tab() === "positions") {
      renderPositions();
    } else {
      renderOrders(tab() === "pending" ? "pending" : "history");
    }
  });

  function renderPositions(): void {
    const positions = account.positions();
    if (positions.length === 0) {
      host.append(list, emptyHint("暂无持仓，去行情页买入"));
      return;
    }
    for (const pos of positions) {
      host.append(list, buildPositionRow(pos.symbol, pos.qty, pos.avgCost));
    }
  }

  function renderOrders(kind: "pending" | "history"): void {
    const orders = account.orders().filter((o) =>
      kind === "pending" ? o.status === "pending" : o.status !== "pending",
    );
    if (orders.length === 0) {
      host.append(list, emptyHint(kind === "pending" ? "没有待成交委托" : "暂无历史委托"));
      return;
    }
    for (const o of orders) {
      host.append(list, buildOrderRow(o.symbol, o.side, o.type, o.price, o.qty, o.status, o.id));
    }
  }

  function emptyHint(text: string): number {
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 0, 0, 24, 0);
    const label = host.text(text, 13, MUTED);
    const align = host.align("center");
    host.append(align, label);
    host.append(pad, align);
    return pad;
  }

  function buildPositionRow(symbol: string, qty: number, avgCost: number): number {
    const ins = engine.ins(symbol);
    const row = host.pointerRegion();
    host.onClick(row, () => props.router.push(`/stock/${symbol}`));
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 10, 0, 10, 0);
    host.append(row, pad);
    const line = host.row(10);
    host.setMinSize(line, 0, 48);
    host.append(pad, line);

    const mono = Monogram({ ins: () => ins });
    host.append(line, mono.id);
    const nameCol = host.column(1);
    host.setGrow(nameCol, 1);
    host.append(line, nameCol);
    const name = host.text(ins.name, 14, INK);
    host.append(nameCol, name);
    const meta = host.text(`${fmtQty(qty)}股 · 成本 ${fmtPrice(avgCost)}`, 11, MUTED);
    host.append(nameCol, meta);

    const pnlCol = host.column(2);
    host.append(line, pnlCol);
    const market = host.text("—", 14, INK);
    host.setFontWeight(market, 600);
    host.append(pnlCol, market);
    const pnl = host.text("—", 11, MUTED);
    host.append(pnlCol, pnl);

    onCleanup(binder.bind(symbol, { last: market, flashNode: market }));
    onCleanup(
      onFrame(() => {
        const last = engine.quote(symbol).last;
        const diff = (last - avgCost) * qty;
        host.setText(pnl, `${diff >= 0 ? "+" : ""}${fmtPrice(diff)}`);
        host.setColor(pnl, diff > 0 ? colorBytes("#EB3E3E") : diff < 0 ? colorBytes("#22B05F") : MUTED);
      }),
    );
    return row;
  }

  function buildOrderRow(
    symbol: string,
    side: "buy" | "sell",
    type: "market" | "limit",
    price: number,
    qty: number,
    status: string,
    id: string,
  ): number {
    const ins = engine.ins(symbol);
    const row = host.pointerRegion();
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 10, 0, 10, 0);
    host.append(row, pad);
    const line = host.row(10);
    host.setMinSize(line, 0, 40);
    host.append(pad, line);

    const sideBadge = host.decorated(side === "buy" ? colorBytes("#EB3E3E") : colorBytes("#22B05F"), 4);
    host.setMinSize(sideBadge, 4, 32);
    host.append(line, sideBadge);
    const nameCol = host.column(1);
    host.setGrow(nameCol, 1);
    host.append(line, nameCol);
    const name = host.text(`${ins.name} ${side === "buy" ? "买" : "卖"}`, 13, INK);
    host.append(nameCol, name);
    const meta = host.text(
      `${type === "market" ? "市价" : "限价 " + fmtPrice(price)} · ${fmtQty(qty)}股 · ${statusLabel(status)}`,
      11,
      MUTED,
    );
    host.append(nameCol, meta);

    if (status === "pending") {
      const cancelBtn = Button({
        label: "撤单",
        onClick: () => account.cancel(id),
        variant: "quiet",
        size: "compact",
      });
      host.append(line, cancelBtn.id);
    }
    return row;
  }

  void settingsSig;
  void QuoteText;
  void PRIMARY;
  void binder;
  return { id: page };
}

function tabLabel(t: Tab): string {
  return t === "positions" ? "持仓" : t === "pending" ? "待成交" : "历史";
}

function statusLabel(status: string): string {
  if (status === "pending") {
    return "待成交";
  }
  if (status === "filled") {
    return "已成交";
  }
  if (status === "cancelled") {
    return "已撤单";
  }
  return "已拒绝";
}

export type { QuoteBinder, QuoteEngine };
