import { createRenderEffect, createSignal, onCleanup } from "solid-js";
import {
  scene,
  insert,
  onLayout,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { ScrollArea, createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { settingsSig } from "../state/settings.ts";
import { fmtPrice, fmtVolumeCN } from "../format.ts";
import { watchlistSig, toggleWatch } from "../state/watchlist.ts";
import type { Router } from "@impasto/solid-router";
import { MinutesChart } from "../widgets/chart/minutes.ts";
import { DepthLadder } from "../widgets/chart/depth.ts";
import { CandleChart } from "../widgets/chart/candles.ts";
import { loc } from "../copy.ts";
import { setOrderTicket } from "../state/order-ticket.ts";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const PRIMARY = colorBytes("#C8102E");
export function StockPage(props: {
  router: Router;
  symbol: string;
}): NodeHandle {
  const { engine, binder } = useQuotes();
  const symbol = () => props.symbol;
  const candles = createScrollController();
  const [mode, setMode] = createSignal<"minutes" | "daily">("minutes");
  const [chartW, setChartW] = createSignal(0);
  const page = scene.createElement("view", {
    style: {
      gap: 0,
    },
  });
  scene.patchStyle(page, {
    flex: 1,
  });
  const body = scene.createElement("view", {
    style: {
      gap: 10,
    },
  });
  const header = scene.createElement("view", {
    style: {
      gap: 6,
    },
  });
  const headerPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(headerPad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 8,
    paddingLeft: 16,
  });
  scene.append(headerPad, header);
  scene.append(body, headerPad);
  const nav = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  scene.append(header, nav);
  const back = scene.createElement("view", {
    style: {},
  });
  scene.onClick(back, () => props.router.pop());
  scene.append(nav, back);
  const backLabel = scene.createElement("text", {
    value: loc("‹ 返回", "‹ Back"),
    style: {
      fontSize: 14,
      color: PRIMARY,
    },
  });
  scene.append(back, backLabel);
  const spacer = scene.createElement("view", {
    style: {
      width: 0,
      height: 0,
    },
  });
  scene.patchStyle(spacer, {
    flex: 1,
  });
  scene.append(nav, spacer);
  const star = scene.createElement("view", {
    style: {},
  });
  scene.onClick(star, () => toggleWatch(symbol()));
  scene.append(nav, star);
  const starLabel = scene.createElement("text", {
    value: "☆",
    style: {
      fontSize: 16,
      color: MUTED,
    },
  });
  scene.append(star, starLabel);
  createRenderEffect(
    () => watchlistSig().includes(symbol()),
    (watched) => {
      scene.setText(starLabel, watched ? "★" : "☆");
    },
  );
  const ins = engine.ins(symbol());
  const nameRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 6,
    },
  });
  scene.append(header, nameRow);
  const name = scene.createElement("text", {
    value: ins.name,
    style: {
      fontSize: 20,
      color: INK,
    },
  });
  scene.patchStyle(name, {
    fontWeight: 700,
  });
  scene.append(nameRow, name);
  const sym = scene.createElement("text", {
    value: ins.symbol,
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(nameRow, sym);
  const bigCol = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 10,
    },
  });
  scene.append(header, bigCol);
  const bigLast = scene.createElement("text", {
    value: "—",
    style: {
      fontSize: 34,
      color: INK,
    },
  });
  scene.patchStyle(bigLast, {
    fontWeight: 700,
  });
  onCleanup(
    binder.bind(symbol(), {
      last: bigLast,
      flashNode: bigLast,
    }),
  );
  scene.append(bigCol, bigLast);
  const bigPctCol = scene.createElement("view", {
    style: {
      gap: 2,
    },
  });
  scene.append(bigCol, bigPctCol);
  const bigPct = scene.createElement("text", {
    value: "—",
    style: {
      fontSize: 14,
      color: MUTED,
    },
  });
  scene.append(bigPctCol, bigPct);
  const bigChg = scene.createElement("text", {
    value: "—",
    style: {
      fontSize: 12,
      color: MUTED,
    },
  });
  scene.append(bigPctCol, bigChg);
  onCleanup(
    binder.bind(symbol(), {
      pct: bigPct,
      chg: bigChg,
      flashNode: bigPct,
    }),
  );
  const statLine = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 10,
    },
  });
  scene.append(header, statLine);
  const statOpen = buildStat(loc("今开", "Open"), () =>
    fmtPrice(engine.quote(symbol()).open),
  );
  const statHigh = buildStat(loc("最高", "High"), () =>
    fmtPrice(engine.quote(symbol()).high),
  );
  const statLow = buildStat(loc("最低", "Low"), () =>
    fmtPrice(engine.quote(symbol()).low),
  );
  const statVol = buildStat(loc("成交量", "Volume"), () =>
    fmtVolumeCN(engine.quote(symbol()).volume),
  );
  for (const s of [statOpen, statHigh, statLow, statVol]) {
    scene.patchStyle(s, {
      flex: 1,
    });
    scene.append(statLine, s);
  }
  function buildStat(labelText: string, read: () => string): number {
    const col = scene.createElement("view", {
      style: {
        gap: 1,
      },
    });
    const cap = scene.createElement("text", {
      value: labelText,
      style: {
        fontSize: 10,
        color: MUTED,
      },
    });
    scene.append(col, cap);
    const val = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 12,
        color: INK,
      },
    });
    scene.append(col, val);
    createRenderEffect(
      () => read(),
      (text) => {
        scene.setText(val, text);
      },
    );
    return col;
  }
  const modeRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 0,
    },
  });
  const modePad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(modePad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(modePad, modeRow);
  scene.append(body, modePad);
  for (const m of ["minutes", "daily"] as const) {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, () => setMode(m));
    scene.append(modeRow, cell);
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
      value: m === "minutes" ? loc("分时", "Intraday") : loc("日K", "Daily"),
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
    createRenderEffect(
      () => mode() === m,
      (active) => {
        scene.patchStyle(label, {
          color: active ? INK : MUTED,
        });
        scene.patchStyle(label, {
          fontWeight: active ? 700 : 400,
        });
      },
    );
  }
  const chartPanel = scene.createElement("view", {
    style: {
      backgroundColor: PANEL,
      borderRadius: 12,
    },
  });
  scene.patchStyle(chartPanel, {
    outlineWidth: 1,
    outlineColor: HAIR,
  });
  const chartInner = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(chartInner, {
    paddingTop: 12,
    paddingRight: 12,
    paddingBottom: 12,
    paddingLeft: 12,
  });
  const chartPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(chartPad, {
    paddingTop: 0,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(chartPad, chartPanel);
  scene.append(body, chartPad);
  scene.append(chartPanel, chartInner);
  const chartStack = scene.stack();
  scene.patchStyle(chartStack, {
    minWidth: 0,
    minHeight: 200,
  });
  scene.append(chartInner, chartStack);
  onCleanup(
    onLayout(() => {
      const parent = scene.parent(chartStack);
      const box = scene.rect(parent !== 0 ? parent : chartStack);
      if (box.w > 0 && box.w !== chartW()) {
        setChartW(box.w);
      }
    }),
  );
  let chartCleanup: (() => void) | null = null;
  createRenderEffect(
    () => mode(),
    (m) => {
      if (chartCleanup) {
        chartCleanup();
      }
      if (m === "minutes") {
        const chart = MinutesChart({
          symbol,
          engine: () => engine,
          binder: () => binder,
          updown: () => settingsSig().updown,
          h: 200,
          viewW: () => chartW(),
        });
        scene.append(chartStack, chart);
        chartCleanup = () => scene.remove(chartStack, chart);
      } else {
        const chart = CandleChart({
          symbol,
          engine: () => engine,
          controller: candles,
          plotH: 150,
          volH: 50,
          viewW: () => chartW(),
          barLabel: (i) => `D${i + 1}`,
        });
        scene.append(chartStack, chart);
        chartCleanup = () => scene.remove(chartStack, chart);
      }
    },
  );
  const depthPanel = scene.createElement("view", {
    style: {
      backgroundColor: PANEL,
      borderRadius: 12,
    },
  });
  scene.patchStyle(depthPanel, {
    outlineWidth: 1,
    outlineColor: HAIR,
  });
  const depthInner = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(depthInner, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 12,
    paddingLeft: 16,
  });
  const depthPad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(depthPad, {
    paddingTop: 0,
    paddingRight: 16,
    paddingBottom: 0,
    paddingLeft: 16,
  });
  scene.append(depthPad, depthPanel);
  scene.append(body, depthPad);
  scene.append(depthPanel, depthInner);
  const depthCol = scene.createElement("view", {
    style: {
      gap: 8,
    },
  });
  scene.append(depthInner, depthCol);
  const depthTitle = scene.createElement("text", {
    value: loc("盘口 五档", "Order book"),
    style: {
      fontSize: 13,
      color: INK,
    },
  });
  scene.patchStyle(depthTitle, {
    fontWeight: 600,
  });
  scene.append(depthCol, depthTitle);
  const depth = DepthLadder({
    symbol,
    engine: () => engine,
    updown: () => settingsSig().updown,
    onPick: (price) => {
      setOrderTicket({
        price: price.toFixed(2),
      });
      props.router.push(`/order/${symbol()}`);
    },
  });
  scene.append(depthCol, depth.node);
  const scroller = ScrollArea({
    axis: "vertical",
    children: body,
  });
  scene.patchStyle(scroller, {
    flex: 1,
  });
  insert(page, scroller);
  const tradeRow = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 10,
    },
  });
  const tradePad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(tradePad, {
    paddingTop: 12,
    paddingRight: 16,
    paddingBottom: 16,
    paddingLeft: 16,
  });
  scene.append(tradePad, tradeRow);
  scene.append(page, tradePad);
  const buyBtn = buildTradeBtn(loc("买入", "Buy"), "#EB3E3E", () => {
    setOrderTicket({
      side: "buy",
    });
    props.router.push(`/order/${symbol()}`);
  });
  const sellBtn = buildTradeBtn(loc("卖出", "Sell"), "#22B05F", () => {
    setOrderTicket({
      side: "sell",
    });
    props.router.push(`/order/${symbol()}`);
  });
  scene.append(tradeRow, buyBtn);
  scene.append(tradeRow, sellBtn);
  function buildTradeBtn(
    labelText: string,
    color: string,
    onClick: () => void,
  ): number {
    const cell = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(cell, {
      flex: 1,
    });
    scene.onClick(cell, onClick);
    const pill = scene.createElement("view", {
      style: {
        backgroundColor: colorBytes(color),
        borderRadius: 8,
      },
    });
    scene.patchStyle(pill, {
      minWidth: 0,
      minHeight: 44,
    });
    scene.patchStyle(pill, {
      flex: 1,
    });
    scene.append(cell, pill);
    const label = scene.createElement("text", {
      value: labelText,
      style: {
        fontSize: 15,
        color: colorBytes("#FFFFFF"),
      },
    });
    scene.patchStyle(label, {
      fontWeight: 600,
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
    scene.append(pill, align);
    return cell;
  }
  return page;
}
