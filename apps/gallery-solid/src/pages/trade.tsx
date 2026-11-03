import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { host, insert, onFrame, type NodeHandle } from "@impasto/solid-renderer";
import { Button, Dialog } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { clearChildren } from "../widgets/palette.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { accountRef } from "../state/account-ref.ts";
import { showToast } from "../state/toast.ts";
import { fmtPrice, fmtQty } from "../format.ts";
import { loc } from "../copy.ts";
import type { Router } from "@impasto/solid-router";
import { Monogram } from "../widgets/monogram.tsx";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");

type Tab = "positions" | "pending" | "filled";

export function TradePage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const account = accountRef();
  const [tab, setTab] = createSignal<Tab>("positions");
  const [closing, setClosing] = createSignal<string | null>(null);

  const page = host.column(8);
  host.setGrow(page, 1);

  const card = host.decorated(colorBytes("#171B26"), 16);
  const cardInner = host.padding(0);
  host.setPaddingEdges(cardInner, 16, 16, 16, 16);
  host.append(card, cardInner);
  const cardPad = host.padding(0);
  host.setPaddingEdges(cardPad, 12, 16, 0, 16);
  host.append(cardPad, card);
  host.append(page, cardPad);

  const cardTitle = host.text(loc("总资产 (模拟)", "Equity (paper)"), 12, colorBytes("#9AA0A6"));
  host.append(cardInner, cardTitle);
  const equityRow = host.row(8);
  host.append(cardInner, equityRow);
  const equity = host.text("—", 30, colorBytes("#FFFFFF"));
  host.setFontWeight(equity, 700);
  host.append(equityRow, equity);

  const statRow = host.row(12);
  host.append(cardInner, statRow);
  const cashStat = cardStat(loc("可用资金", "Cash"));
  const mvStat = cardStat(loc("持仓市值", "Market value"));
  const pnlStat = cardStat(loc("浮动盈亏", "Unrealized"));
  for (const s of [cashStat, mvStat, pnlStat]) {
    host.setGrow(s.root, 1);
    host.append(statRow, s.root);
  }

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
      host.setColor(
        pnlStat.value,
        pnl > 0 ? colorBytes("#FF7B72") : pnl < 0 ? colorBytes("#3FB950") : colorBytes("#9AA0A6"),
      );
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

  const tabs = host.row(0);
  const tabPad = host.padding(0);
  host.setPaddingEdges(tabPad, 12, 16, 0, 16);
  host.append(tabPad, tabs);
  host.append(page, tabPad);
  for (const t of ["positions", "pending", "filled"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setTab(t));
    host.append(tabs, cell);
    const inner = host.padding(0);
    host.setPaddingEdges(inner, 8, 0, 8, 0);
    host.setGrow(inner, 1);
    host.append(cell, inner);
    const label = host.text(tabLabel(t), 13, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(inner, align);
    createTrackedEffect(() => {
      const active = tab() === t;
      host.setColor(label, active ? INK : MUTED);
      host.setFontWeight(label, active ? 700 : 400);
    });
  }

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
      renderOrders(tab());
    }
  });

  function renderPositions(): void {
    const positions = account.positions();
    if (positions.length === 0) {
      host.append(list, emptyHint(loc("暂无持仓，去行情页买入", "No positions yet")));
      return;
    }
    for (const pos of positions) {
      host.append(list, buildPositionRow(pos.symbol, pos.qty, pos.avgCost));
    }
  }

  function renderOrders(kind: "pending" | "filled"): void {
    const orders = account.orders().filter((o) =>
      kind === "pending" ? o.status === "pending" : o.status === "filled",
    );
    if (orders.length === 0) {
      host.append(
        list,
        emptyHint(kind === "pending" ? loc("没有待成交委托", "No open orders") : loc("暂无已成交", "No fills yet")),
      );
      return;
    }
    for (const o of orders) {
      host.append(list, buildOrderRow(o.symbol, o.side, o.type, o.price, o.qty, o.status, o.id));
    }
  }

  function emptyHint(text: string): number {
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 24, 0, 24, 0);
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
    host.setGrow(pad, 1);
    host.append(row, pad);
    const line = host.row(10);
    host.setMinSize(line, 0, 48);
    host.setCross(line, "center");
    host.append(pad, line);

    const mono = Monogram({ ins: () => ins });
    host.append(line, mono.id);
    const nameCol = host.column(1);
    host.setGrow(nameCol, 1);
    host.append(line, nameCol);
    const name = host.text(ins.name, 14, INK);
    host.append(nameCol, name);
    const meta = host.text(`${fmtQty(qty)}${loc("股", " sh")} · ${loc("成本", "cost")} ${fmtPrice(avgCost)}`, 11, MUTED);
    host.append(nameCol, meta);

    const pnlCol = host.column(2);
    host.append(line, pnlCol);
    const market = host.text("—", 14, INK);
    host.setFontWeight(market, 600);
    host.append(pnlCol, market);
    const pnl = host.text("—", 11, MUTED);
    host.append(pnlCol, pnl);

    const closeBtn = Button({
      label: loc("平仓", "Close"),
      onClick: () => setClosing(symbol),
      variant: "quiet",
      size: "compact",
    });
    host.append(line, closeBtn.id);

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
    host.setGrow(pad, 1);
    host.append(row, pad);
    const line = host.row(10);
    host.setMinSize(line, 0, 40);
    host.setCross(line, "center");
    host.append(pad, line);

    const sideBadge = host.decorated(side === "buy" ? colorBytes("#EB3E3E") : colorBytes("#22B05F"), 4);
    host.setMinSize(sideBadge, 4, 32);
    host.append(line, sideBadge);
    const nameCol = host.column(1);
    host.setGrow(nameCol, 1);
    host.append(line, nameCol);
    const name = host.text(`${ins.name} ${side === "buy" ? loc("买", "Buy") : loc("卖", "Sell")}`, 13, INK);
    host.append(nameCol, name);
    const meta = host.text(
      `${type === "market" ? loc("市价", "Mkt") : loc("限价", "Lmt") + " " + fmtPrice(price)} · ${fmtQty(qty)}${loc("股", " sh")} · ${statusLabel(status)}`,
      11,
      MUTED,
    );
    host.append(nameCol, meta);

    if (status === "pending") {
      const cancelBtn = Button({
        label: loc("撤单", "Cancel"),
        onClick: () => {
          account.cancel(id);
          showToast(loc("已撤单", "Cancelled"));
        },
        variant: "quiet",
        size: "compact",
      });
      host.append(line, cancelBtn.id);
    }
    return row;
  }

  const confirmBody = host.column(8);
  const confirmText = host.text("", 14, INK);
  host.append(confirmBody, confirmText);
  createTrackedEffect(() => {
    const symbol = closing();
    host.setText(
      confirmText,
      symbol ? loc(`确认平掉 ${engine.ins(symbol).name} 全部持仓？`, `Close all ${engine.ins(symbol).name}?`) : "",
    );
  });
  const confirmRow = host.row(8);
  host.append(confirmBody, confirmRow);
  const cancelClose = Button({
    label: loc("取消", "Cancel"),
    onClick: () => setClosing(null),
    variant: "quiet",
    size: "compact",
  });
  host.append(confirmRow, cancelClose.id);
  const okClose = Button({
    label: loc("确认平仓", "Close"),
    onClick: () => {
      const symbol = closing();
      setClosing(null);
      if (!symbol) {
        return;
      }
      const result = account.flatten(symbol);
      if (!result.ok) {
        showToast(result.reason);
        return;
      }
      showToast(loc("已平仓", "Closed"));
    },
    variant: "primary",
    size: "compact",
  });
  host.append(confirmRow, okClose.id);

  const dialog = Dialog({
    get open() {
      return closing() !== null;
    },
    onClose: () => setClosing(null),
    title: loc("平仓", "Close position"),
    children: { id: confirmBody },
  });
  insert({ id: page } as NodeHandle, dialog);

  return { id: page };
}

function tabLabel(t: Tab): string {
  return t === "positions" ? loc("持仓", "Positions") : t === "pending" ? loc("待成交", "Open") : loc("已成交", "Filled");
}

function statusLabel(status: string): string {
  if (status === "pending") {
    return loc("待成交", "Open");
  }
  if (status === "filled") {
    return loc("已成交", "Filled");
  }
  if (status === "cancelled") {
    return loc("已撤单", "Cancelled");
  }
  return loc("已拒绝", "Rejected");
}
