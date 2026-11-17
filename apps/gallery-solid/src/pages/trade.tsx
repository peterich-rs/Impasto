import { createSignal, createTrackedEffect, onCleanup } from "solid-js";
import {
  scene,
  insert,
  onFrame,
  type NodeHandle,
} from "@impasto/solid-renderer";
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
  const page = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  scene.patchStyle(page, {
    flex: 1,
  });
  const card = scene.createElement("view", {
    style: {
      backgroundColor: colorBytes("#171B26"),
      borderRadius: 16,
    },
  });
  const cardInner = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(cardInner, {
    paddingTop: 16,
    paddingRight: 16,
    paddingBottom: 16,
    paddingLeft: 16,
  });
  scene.append(card, cardInner);
  const cardPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(cardPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(cardPad, card);
  scene.append(page, cardPad);
  const cardTitle = scene.createElement("text", {
    value: loc("总资产 (模拟)", "Equity (paper)"),
    style: {
      fontSize: 12,
      color: colorBytes("#9AA0A6"),
    },
  });
  scene.append(cardInner, cardTitle);
  const equityRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(cardInner, equityRow);
  const equity = scene.createElement("text", {
    value: "—",
    style: {
      fontSize: 30,
      color: colorBytes("#FFFFFF"),
    },
  });
  scene.patchStyle(equity, {
    fontWeight: 700,
  });
  scene.append(equityRow, equity);
  const statRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 12,
    },
  });
  scene.append(cardInner, statRow);
  const cashStat = cardStat(loc("可用资金", "Cash"));
  const mvStat = cardStat(loc("持仓市值", "Market value"));
  const pnlStat = cardStat(loc("浮动盈亏", "Unrealized"));
  for (const s of [cashStat, mvStat, pnlStat]) {
    scene.patchStyle(s.root, {
      flex: 1,
    });
    scene.append(statRow, s.root);
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
      const pnl =
        mv - account.positions().reduce((s, p) => s + p.avgCost * p.qty, 0);
      scene.setText(equity, fmtPrice(total));
      scene.setText(cashStat.value, fmtPrice(account.cash()));
      scene.setText(mvStat.value, fmtPrice(mv));
      scene.setText(pnlStat.value, fmtPrice(pnl));
      scene.patchStyle(pnlStat.value, {
        color:
          pnl > 0
            ? colorBytes("#FF7B72")
            : pnl < 0
              ? colorBytes("#3FB950")
              : colorBytes("#9AA0A6"),
      });
    }),
  );
  function cardStat(labelText: string): {
    root: number;
    value: number;
  } {
    const col = scene.createElement("view", {
      style: {
        gap: 2,
      },
    });
    const cap = scene.createElement("text", {
      value: labelText,
      style: {
        fontSize: 11,
        color: colorBytes("#9AA0A6"),
      },
    });
    scene.append(col, cap);
    const value = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 14,
        color: colorBytes("#FFFFFF"),
      },
    });
    scene.patchStyle(value, {
      fontWeight: 600,
    });
    scene.append(col, value);
    return {
      root: col,
      value,
    };
  }
  const tabs = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  const tabPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(tabPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(tabPad, tabs);
  scene.append(page, tabPad);
  for (const t of ["positions", "pending", "filled"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setTab(t));
    scene.append(tabs, cell);
    const inner = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(inner, {
      paddingTop: 8,
      paddingRight: 0,
      paddingBottom: 8,
      paddingLeft: 0,
    });
    scene.patchStyle(inner, {
      flex: 1,
    });
    scene.append(cell, inner);
    const label = scene.createElement("text", {
      value: tabLabel(t),
      style: {
        fontSize: 13,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(align, {
      flex: 1,
    });
    scene.append(align, label);
    scene.append(inner, align);
    createTrackedEffect(() => {
      const active = tab() === t;
      scene.patchStyle(label, {
        color: active ? INK : MUTED,
      });
      scene.patchStyle(label, {
        fontWeight: active ? 700 : 400,
      });
    });
  }
  const panel = scene.createElement("view", {
    style: {
      backgroundColor: PANEL,
      borderRadius: 12,
    },
  });
  scene.patchStyle(panel, {
    outlineWidth: 1,
    outlineColor: HAIR,
  });
  scene.patchStyle(panel, {
    flex: 1,
  });
  const panelPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(panelPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 16,
    paddingLeft: 16,
  });
  scene.append(panelPad, panel);
  scene.append(page, panelPad);
  const list = scene.createElement("view", {
    style: {
      gap: 0,
    },
  });
  scene.append(panel, list);
  createTrackedEffect(() => {
    clearChildren(list);
    if (tab() === "positions") {
      renderPositions();
    } else {
      renderOrders(tab() as "pending" | "filled");
    }
  });
  function renderPositions(): void {
    const positions = account.positions();
    if (positions.length === 0) {
      scene.append(
        list,
        emptyHint(loc("暂无持仓，去行情页买入", "No positions yet")),
      );
      return;
    }
    for (const pos of positions) {
      scene.append(list, buildPositionRow(pos.symbol, pos.qty, pos.avgCost));
    }
  }
  function renderOrders(kind: "pending" | "filled"): void {
    const orders = account
      .orders()
      .filter((o) =>
        kind === "pending" ? o.status === "pending" : o.status === "filled",
      );
    if (orders.length === 0) {
      scene.append(
        list,
        emptyHint(
          kind === "pending"
            ? loc("没有待成交委托", "No open orders")
            : loc("暂无已成交", "No fills yet"),
        ),
      );
      return;
    }
    for (const o of orders) {
      scene.append(
        list,
        buildOrderRow(o.symbol, o.side, o.type, o.price, o.qty, o.status, o.id),
      );
    }
  }
  function emptyHint(text: string): number {
    const pad = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(pad, {
      paddingTop: 24,
      paddingRight: 0,
      paddingBottom: 24,
      paddingLeft: 0,
    });
    const label = scene.createElement("text", {
      value: text,
      style: {
        fontSize: 13,
        color: MUTED,
      },
    });
    const align = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.append(align, label);
    scene.append(pad, align);
    return pad;
  }
  function buildPositionRow(
    symbol: string,
    qty: number,
    avgCost: number,
  ): number {
    const ins = engine.ins(symbol);
    const row = scene.createElement("view", {
      style: {},
    });
    scene.onClick(row, () => props.router.push(`/stock/${symbol}`));
    const pad = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(pad, {
      paddingTop: 10,
      paddingRight: 0,
      paddingBottom: 10,
      paddingLeft: 0,
    });
    scene.patchStyle(pad, {
      flex: 1,
    });
    scene.append(row, pad);
    const line = scene.createElement("view", {
      style: {
        flexDirection: "row",
        gap: 10,
      },
    });
    scene.patchStyle(line, {
      minWidth: 0,
      minHeight: 48,
    });
    scene.patchStyle(line, {
      alignItems: "center",
    });
    scene.append(pad, line);
    const mono = Monogram({
      ins: () => ins,
    });
    scene.append(line, mono);
    const nameCol = scene.createElement("view", {
      style: {
        gap: 1,
      },
    });
    scene.patchStyle(nameCol, {
      flex: 1,
    });
    scene.append(line, nameCol);
    const name = scene.createElement("text", {
      value: ins.name,
      style: {
        fontSize: 14,
        color: INK,
      },
    });
    scene.append(nameCol, name);
    const meta = scene.createElement("text", {
      value: `${fmtQty(qty)}${loc("股", " sh")} · ${loc("成本", "cost")} ${fmtPrice(avgCost)}`,
      style: {
        fontSize: 11,
        color: MUTED,
      },
    });
    scene.append(nameCol, meta);
    const pnlCol = scene.createElement("view", {
      style: {
        gap: 2,
      },
    });
    scene.append(line, pnlCol);
    const market = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 14,
        color: INK,
      },
    });
    scene.patchStyle(market, {
      fontWeight: 600,
    });
    scene.append(pnlCol, market);
    const pnl = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 11,
        color: MUTED,
      },
    });
    scene.append(pnlCol, pnl);
    const closeBtn = Button({
      label: loc("平仓", "Close"),
      onClick: () => setClosing(symbol),
      variant: "quiet",
      size: "compact",
    });
    scene.append(line, closeBtn);
    onCleanup(
      binder.bind(symbol, {
        last: market,
        flashNode: market,
      }),
    );
    onCleanup(
      onFrame(() => {
        const last = engine.quote(symbol).last;
        const diff = (last - avgCost) * qty;
        scene.setText(pnl, `${diff >= 0 ? "+" : ""}${fmtPrice(diff)}`);
        scene.patchStyle(pnl, {
          color:
            diff > 0
              ? colorBytes("#EB3E3E")
              : diff < 0
                ? colorBytes("#22B05F")
                : MUTED,
        });
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
    const row = scene.createElement("view", {
      style: {},
    });
    const pad = scene.createElement("view", {
      style: {
        padding: 0,
      },
    });
    scene.patchStyle(pad, {
      paddingTop: 10,
      paddingRight: 0,
      paddingBottom: 10,
      paddingLeft: 0,
    });
    scene.patchStyle(pad, {
      flex: 1,
    });
    scene.append(row, pad);
    const line = scene.createElement("view", {
      style: {
        flexDirection: "row",
        gap: 10,
      },
    });
    scene.patchStyle(line, {
      minWidth: 0,
      minHeight: 40,
    });
    scene.patchStyle(line, {
      alignItems: "center",
    });
    scene.append(pad, line);
    const sideBadge = scene.createElement("view", {
      style: {
        backgroundColor:
          side === "buy" ? colorBytes("#EB3E3E") : colorBytes("#22B05F"),
        borderRadius: 4,
      },
    });
    scene.patchStyle(sideBadge, {
      minWidth: 4,
      minHeight: 32,
    });
    scene.append(line, sideBadge);
    const nameCol = scene.createElement("view", {
      style: {
        gap: 1,
      },
    });
    scene.patchStyle(nameCol, {
      flex: 1,
    });
    scene.append(line, nameCol);
    const name = scene.createElement("text", {
      value: `${ins.name} ${side === "buy" ? loc("买", "Buy") : loc("卖", "Sell")}`,
      style: {
        fontSize: 13,
        color: INK,
      },
    });
    scene.append(nameCol, name);
    const meta = scene.createElement("text", {
      value: `${type === "market" ? loc("市价", "Mkt") : loc("限价", "Lmt") + " " + fmtPrice(price)} · ${fmtQty(qty)}${loc("股", " sh")} · ${statusLabel(status)}`,
      style: {
        fontSize: 11,
        color: MUTED,
      },
    });
    scene.append(nameCol, meta);
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
      scene.append(line, cancelBtn);
    }
    return row;
  }
  const confirmBody = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  const confirmText = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 14,
      color: INK,
    },
  });
  scene.append(confirmBody, confirmText);
  createTrackedEffect(() => {
    const symbol = closing();
    scene.setText(
      confirmText,
      symbol
        ? loc(
            `确认平掉 ${engine.ins(symbol).name} 全部持仓？`,
            `Close all ${engine.ins(symbol).name}?`,
          )
        : "",
    );
  });
  const confirmRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(confirmBody, confirmRow);
  const cancelClose = Button({
    label: loc("取消", "Cancel"),
    onClick: () => setClosing(null),
    variant: "quiet",
    size: "compact",
  });
  scene.append(confirmRow, cancelClose);
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
  scene.append(confirmRow, okClose);
  const dialog = Dialog({
    get open() {
      return closing() !== null;
    },
    onClose: () => setClosing(null),
    title: loc("平仓", "Close position"),
    children: confirmBody,
  });
  insert(page, dialog);
  return page;
}
function tabLabel(t: Tab): string {
  return t === "positions"
    ? loc("持仓", "Positions")
    : t === "pending"
      ? loc("待成交", "Open")
      : loc("已成交", "Filled");
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
