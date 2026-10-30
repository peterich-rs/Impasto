import { createRenderEffect, createSignal, onCleanup } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { settingsSig, toneFor } from "../state/settings.ts";
import { fmtPct, fmtPrice, fmtVolumeCN } from "../format.ts";
import { watchlistSig, toggleWatch } from "../state/watchlist.ts";
import type { Router } from "@impasto/solid-router";
import { MinutesChart } from "../widgets/chart/minutes.ts";
import { DepthLadder } from "../widgets/chart/depth.ts";
import { CandleChart } from "../widgets/chart/candles.ts";
import { fmtQty } from "../format.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");
const PRIMARY = colorBytes("#C8102E");

const VIEW_W = 328;

export function StockPage(props: { router: Router; symbol: string }): NodeHandle {
  const { engine, binder } = useQuotes();
  const symbol = () => props.symbol;
  const controller = createScrollController();
  const [mode, setMode] = createSignal<"minutes" | "daily">("minutes");
  const [tickSize, setTickSize] = createSignal(0.01);

  const page = host.column(0);
  host.setGrow(page, 1);

  // === header ===
  const header = host.column(6);
  const headerPad = host.padding(0);
  host.setPaddingEdges(headerPad, 12, 16, 8, 16);
  host.append(headerPad, header);
  host.append(page, headerPad);

  const nav = host.row(8);
  host.append(header, nav);
  const back = host.pointerRegion();
  host.onClick(back, () => props.router.pop());
  host.append(nav, back);
  const backLabel = host.text("‹ 返回", 14, PRIMARY);
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

  // big last / pct, flash via binder
  const bigCol = host.row(10);
  host.append(header, bigCol);
  const bigLast = QuoteTextBig();
  function QuoteTextBig(): number {
    const node = host.text("—", 34, INK);
    host.setFontWeight(node, 700);
    onCleanup(binder.bind(symbol(), { last: node, flashNode: node }));
    return node;
  }
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
  const statOpen = buildStat("今开", () => fmtPrice(engine.quote(symbol()).open));
  const statHigh = buildStat("最高", () => fmtPrice(engine.quote(symbol()).high));
  const statLow = buildStat("最低", () => fmtPrice(engine.quote(symbol()).low));
  const statVol = buildStat("成交量", () => fmtVolumeCN(engine.quote(symbol()).volume));
  for (const s of [statOpen, statHigh, statLow, statVol]) {
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

  // === chart mode tabs ===
  const modeRow = host.row(0);
  const modePad = host.padding(0);
  host.setPaddingEdges(modePad, 12, 16, 0, 16);
  host.append(modePad, modeRow);
  host.append(page, modePad);
  for (const m of ["minutes", "daily"] as const) {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setMode(m));
    host.append(modeRow, cell);
    const inner = host.padding(0);
    host.setPaddingEdges(inner, 6, 0, 6, 0);
    host.append(cell, inner);
    const label = host.text(m === "minutes" ? "分时" : "日K", 13, MUTED);
    const align = host.align("center");
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

  // === chart card ===
  const chartPanel = host.decorated(PANEL, 12);
  host.setStroke(chartPanel, 1, HAIR);
  const chartInner = host.padding(0);
  host.setPaddingEdges(chartInner, 12, 12, 12, 12);
  host.append(page, chartPanel);
  host.append(chartPanel, chartInner);

  const chartStack = host.stack();
  host.setMinSize(chartStack, VIEW_W, 200);
  host.append(chartInner, chartStack);

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
        });
        host.append(chartStack, chart.id);
        chartCleanup = () => host.remove(chartStack, chart.id);
      } else {
        const chart = CandleChart({
          symbol,
          engine: () => engine,
          controller,
          plotH: 150,
          volH: 50,
          viewW: () => VIEW_W,
          barLabel: (i) => `D${i + 1}`,
        });
        host.append(chartStack, chart.id);
        chartCleanup = () => host.remove(chartStack, chart.id);
      }
    },
  );

  // === depth ladder ===
  const depthPanel = host.decorated(PANEL, 12);
  host.setStroke(depthPanel, 1, HAIR);
  const depthInner = host.padding(0);
  host.setPaddingEdges(depthInner, 12, 16, 12, 16);
  host.append(page, depthPanel);
  host.append(depthPanel, depthInner);
  const depthTitle = host.text("盘口 五档", 13, INK);
  host.setFontWeight(depthTitle, 600);
  host.append(depthInner, depthTitle);
  const depth = DepthLadder({
    symbol,
    engine: () => engine,
    updown: () => settingsSig().updown,
    onPick: (price) => props.router.push(`/order/${symbol()}?price=${price.toFixed(2)}`),
  });
  host.append(depthInner, depth.id);

  // === trade buttons ===
  const tradeRow = host.row(10);
  const tradePad = host.padding(0);
  host.setPaddingEdges(tradePad, 12, 16, 16, 16);
  host.append(tradePad, tradeRow);
  host.append(page, tradePad);
  const buyBtn = buildTradeBtn("买入", "#EB3E3E", () => props.router.push(`/order/${symbol()}?side=buy`));
  const sellBtn = buildTradeBtn("卖出", "#22B05F", () => props.router.push(`/order/${symbol()}?side=sell`));
  host.append(tradeRow, buyBtn);
  host.append(tradeRow, sellBtn);

  function buildTradeBtn(labelText: string, color: string, onClick: () => void): number {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, onClick);
    const pill = host.decorated(colorBytes(color), 999);
    host.setMinSize(pill, 0, 44);
    host.append(cell, pill);
    const label = host.text(labelText, 15, colorBytes("#FFFFFF"));
    host.setFontWeight(label, 600);
    const align = host.align("center");
    host.append(align, label);
    host.append(pill, align);
    return cell;
  }

  void tickSize;
  void setTickSize;
  void toneFor;
  void fmtQty;
  void fmtPct;
  void controller;
  return { id: page };
}
