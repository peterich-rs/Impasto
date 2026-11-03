import { createRenderEffect, createSignal, onCleanup } from "solid-js";
import { host, insert, onLayout, type NodeHandle } from "@impasto/solid-renderer";
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

export function StockPage(props: { router: Router; symbol: string }): NodeHandle {
  const { engine, binder } = useQuotes();
  const symbol = () => props.symbol;
  const candles = createScrollController();
  const [mode, setMode] = createSignal<"minutes" | "daily">("minutes");
  const [chartW, setChartW] = createSignal(0);

  const page = host.column(0);
  host.setGrow(page, 1);

  const body = host.column(10);

  const header = host.column(6);
  const headerPad = host.padding(0);
  host.setPaddingEdges(headerPad, 12, 16, 8, 16);
  host.append(headerPad, header);
  host.append(body, headerPad);

  const nav = host.row(8);
  host.append(header, nav);
  const back = host.pointerRegion();
  host.onClick(back, () => props.router.pop());
  host.append(nav, back);
  const backLabel = host.text(loc("‹ 返回", "‹ Back"), 14, PRIMARY);
  host.append(back, backLabel);
  const spacer = host.sized(0, 0);
  host.setGrow(spacer, 1);
  host.append(nav, spacer);
  const star = host.pointerRegion();
  host.onClick(star, () => toggleWatch(symbol()));
  host.append(nav, star);
  const starLabel = host.text("☆", 16, MUTED);
  host.append(star, starLabel);
  createRenderEffect(
    () => watchlistSig().includes(symbol()),
    (watched) => {
      host.setText(starLabel, watched ? "★" : "☆");
    },
  );

  const ins = engine.ins(symbol());
  const nameRow = host.row(6);
  host.append(header, nameRow);
  const name = host.text(ins.name, 20, INK);
  host.setFontWeight(name, 700);
  host.append(nameRow, name);
  const sym = host.text(ins.symbol, 12, MUTED);
  host.append(nameRow, sym);

  const bigCol = host.row(10);
  host.append(header, bigCol);
  const bigLast = host.text("—", 34, INK);
  host.setFontWeight(bigLast, 700);
  onCleanup(binder.bind(symbol(), { last: bigLast, flashNode: bigLast }));
  host.append(bigCol, bigLast);
  const bigPctCol = host.column(2);
  host.append(bigCol, bigPctCol);
  const bigPct = host.text("—", 14, MUTED);
  host.append(bigPctCol, bigPct);
  const bigChg = host.text("—", 12, MUTED);
  host.append(bigPctCol, bigChg);
  onCleanup(binder.bind(symbol(), { pct: bigPct, chg: bigChg, flashNode: bigPct }));

  const statLine = host.row(10);
  host.append(header, statLine);
  const statOpen = buildStat(loc("今开", "Open"), () => fmtPrice(engine.quote(symbol()).open));
  const statHigh = buildStat(loc("最高", "High"), () => fmtPrice(engine.quote(symbol()).high));
  const statLow = buildStat(loc("最低", "Low"), () => fmtPrice(engine.quote(symbol()).low));
  const statVol = buildStat(loc("成交量", "Volume"), () => fmtVolumeCN(engine.quote(symbol()).volume));
  for (const s of [statOpen, statHigh, statLow, statVol]) {
    host.setGrow(s, 1);
    host.append(statLine, s);
  }

  function buildStat(labelText: string, read: () => string): number {
    const col = host.column(1);
    const cap = host.text(labelText, 10, MUTED);
    host.append(col, cap);
    const val = host.text("—", 12, INK);
    host.append(col, val);
    createRenderEffect(
      () => read(),
      (text) => {
        host.setText(val, text);
      },
    );
    return col;
  }

  const modeRow = host.row(0);
  const modePad = host.padding(0);
  host.setPaddingEdges(modePad, 12, 16, 0, 16);
  host.append(modePad, modeRow);
  host.append(body, modePad);
  for (const m of ["minutes", "daily"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setMode(m));
    host.append(modeRow, cell);
    const inner = host.padding(0);
    host.setPaddingEdges(inner, 8, 0, 8, 0);
    host.setGrow(inner, 1);
    host.append(cell, inner);
    const label = host.text(m === "minutes" ? loc("分时", "Intraday") : loc("日K", "Daily"), 13, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(inner, align);
    createRenderEffect(
      () => mode() === m,
      (active) => {
        host.setColor(label, active ? INK : MUTED);
        host.setFontWeight(label, active ? 700 : 400);
      },
    );
  }

  const chartPanel = host.decorated(PANEL, 12);
  host.setStroke(chartPanel, 1, HAIR);
  const chartInner = host.padding(0);
  host.setPaddingEdges(chartInner, 12, 12, 12, 12);
  const chartPad = host.padding(0);
  host.setPaddingEdges(chartPad, 0, 16, 0, 16);
  host.append(chartPad, chartPanel);
  host.append(body, chartPad);
  host.append(chartPanel, chartInner);

  const chartStack = host.stack();
  host.setMinSize(chartStack, 0, 200);
  host.append(chartInner, chartStack);

  onCleanup(
    onLayout(() => {
      const parent = host.parent(chartStack);
      const box = host.rect(parent !== 0 ? parent : chartStack);
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
        host.append(chartStack, chart.id);
        chartCleanup = () => host.remove(chartStack, chart.id);
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
        host.append(chartStack, chart.id);
        chartCleanup = () => host.remove(chartStack, chart.id);
      }
    },
  );

  const depthPanel = host.decorated(PANEL, 12);
  host.setStroke(depthPanel, 1, HAIR);
  const depthInner = host.padding(0);
  host.setPaddingEdges(depthInner, 12, 16, 12, 16);
  const depthPad = host.padding(0);
  host.setPaddingEdges(depthPad, 0, 16, 0, 16);
  host.append(depthPad, depthPanel);
  host.append(body, depthPad);
  host.append(depthPanel, depthInner);
  const depthCol = host.column(8);
  host.append(depthInner, depthCol);
  const depthTitle = host.text(loc("盘口 五档", "Order book"), 13, INK);
  host.setFontWeight(depthTitle, 600);
  host.append(depthCol, depthTitle);
  const depth = DepthLadder({
    symbol,
    engine: () => engine,
    updown: () => settingsSig().updown,
    onPick: (price) => {
      setOrderTicket({ price: price.toFixed(2) });
      props.router.push(`/order/${symbol()}`);
    },
  });
  host.append(depthCol, depth.id);

  const scroller = ScrollArea({
    axis: "vertical",
    children: { id: body },
  });
  host.setGrow(scroller.id, 1);
  insert({ id: page } as NodeHandle, scroller);

  const tradeRow = host.row(10);
  const tradePad = host.padding(0);
  host.setPaddingEdges(tradePad, 12, 16, 16, 16);
  host.append(tradePad, tradeRow);
  host.append(page, tradePad);
  const buyBtn = buildTradeBtn(loc("买入", "Buy"), "#EB3E3E", () => {
    setOrderTicket({ side: "buy" });
    props.router.push(`/order/${symbol()}`);
  });
  const sellBtn = buildTradeBtn(loc("卖出", "Sell"), "#22B05F", () => {
    setOrderTicket({ side: "sell" });
    props.router.push(`/order/${symbol()}`);
  });
  host.append(tradeRow, buyBtn);
  host.append(tradeRow, sellBtn);

  function buildTradeBtn(labelText: string, color: string, onClick: () => void): number {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, onClick);
    const pill = host.decorated(colorBytes(color), 8);
    host.setMinSize(pill, 0, 44);
    host.setGrow(pill, 1);
    host.append(cell, pill);
    const label = host.text(labelText, 15, colorBytes("#FFFFFF"));
    host.setFontWeight(label, 600);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(pill, align);
    return cell;
  }

  return { id: page };
}
