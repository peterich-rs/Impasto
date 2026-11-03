import { createRenderEffect, createMemo, createSignal } from "solid-js";
import { host, insert, type NodeHandle } from "@impasto/solid-renderer";
import { VirtualList, createScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import type { Router } from "@impasto/solid-router";
import { QuoteText } from "../widgets/quote-text.tsx";
import { Monogram } from "../widgets/monogram.tsx";
import { IndexStrip } from "../widgets/index-strip.tsx";
import { loc } from "../copy.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const PANEL = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");

type Board = "gainers" | "losers" | "turnover";
type BoardItem = { symbol: string };

export function MarketPage(props: { router: Router }): NodeHandle {
  const { engine, binder } = useQuotes();
  const [tab, setTab] = createSignal<Board>("gainers");
  const [market, setMarket] = createSignal<"hk" | "us" | "cn">("hk");
  const controller = createScrollController();

  const page = host.column(10);
  host.setGrow(page, 1);

  const title = host.text(loc("市场", "Market"), 26, INK);
  host.setFontWeight(title, 700);
  const titlePad = host.padding(0);
  host.setPaddingEdges(titlePad, 12, 16, 0, 16);
  host.append(titlePad, title);
  host.append(page, titlePad);

  const stripHost = IndexStrip({
    symbols: () => engine.instruments().filter((ins) => ins.index).map((ins) => ins.symbol),
    engine: () => engine,
    binder: () => binder,
    onClick: (symbol) => props.router.push(`/stock/${symbol}`),
  });
  const stripPad = host.padding(0);
  host.setPaddingEdges(stripPad, 8, 16, 0, 16);
  host.append(stripPad, stripHost.id);
  host.append(page, stripPad);

  const pills = host.row(6);
  const pillPad = host.padding(0);
  host.setPaddingEdges(pillPad, 12, 16, 0, 16);
  host.append(pillPad, pills);
  host.append(page, pillPad);
  for (const m of ["hk", "us", "cn"] as const) {
    buildPill(m);
  }

  function buildPill(m: "hk" | "us" | "cn"): void {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setMarket(m));
    host.append(pills, cell);
    const pill = host.decorated(PANEL, 999);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 32);
    host.setGrow(pill, 1);
    host.append(cell, pill);
    const label = host.text(labelOf(m), 12, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(pill, align);
    createRenderEffect(
      () => market() === m,
      (active) => {
        host.setColor(pill, active ? INK : PANEL);
        host.setColor(label, active ? colorBytes("#FFFFFF") : MUTED);
      },
    );
  }

  const tabs = host.row(0);
  const tabPad = host.padding(0);
  host.setPaddingEdges(tabPad, 12, 16, 0, 16);
  host.append(tabPad, tabs);
  host.append(page, tabPad);
  for (const b of ["gainers", "losers", "turnover"] as const) {
    buildTab(b);
  }

  function buildTab(b: Board): void {
    const cell = host.pointerRegion();
    host.setGrow(cell, 1);
    host.onClick(cell, () => setTab(b));
    host.append(tabs, cell);
    const inner = host.padding(0);
    host.setPaddingEdges(inner, 8, 0, 8, 0);
    host.setGrow(inner, 1);
    host.append(cell, inner);
    const label = host.text(boardLabel(b), 13, MUTED);
    const align = host.align("center");
    host.setGrow(align, 1);
    host.append(align, label);
    host.append(inner, align);
    createRenderEffect(
      () => tab() === b,
      (active) => {
        host.setColor(label, active ? INK : MUTED);
        host.setFontWeight(label, active ? 700 : 400);
      },
    );
  }

  const items = createMemo<BoardItem[]>(() => {
    const all = engine.instruments().filter((ins) => ins.market === market() && !ins.index);
    const pct = (s: string) => {
      const q = engine.quote(s);
      return (q.last - q.prevClose) / q.prevClose;
    };
    if (tab() === "turnover") {
      return all
        .map((ins) => ({ symbol: ins.symbol }))
        .sort((a, b) => engine.quote(b.symbol).volume - engine.quote(a.symbol).volume)
        .slice(0, 20);
    }
    return all
      .map((ins) => ({ symbol: ins.symbol }))
      .sort((a, b) => (tab() === "gainers" ? pct(b.symbol) - pct(a.symbol) : pct(a.symbol) - pct(b.symbol)))
      .slice(0, 20);
  });

  const panel = host.decorated(PANEL, 12);
  host.setStroke(panel, 1, HAIR);
  host.setGrow(panel, 1);
  host.append(page, panel);

  const list = VirtualList({
    get items() {
      return items();
    },
    itemKey: (item: BoardItem) => item.symbol,
    estimatedHeight: 64,
    restoreKey: "market",
    controller,
    recycle: {
      typeOf: () => "row",
      render: (item, index) => ({ id: buildBoardRow(item(), index()) }),
    },
  });
  insert({ id: panel } as NodeHandle, list);

  function buildBoardRow(item: BoardItem, index: number): number {
    const ins = engine.ins(item.symbol);
    const row = host.pointerRegion();
    host.onClick(row, () => props.router.push(`/stock/${item.symbol}`));
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 10, 16, 10, 16);
    host.setGrow(pad, 1);
    host.append(row, pad);
    const line = host.row(8);
    host.setMinSize(line, 0, 44);
    host.setCross(line, "center");
    host.append(pad, line);

    const rank = host.text(String(index + 1), 12, MUTED);
    host.setMinSize(rank, 18, 0);
    host.append(line, rank);

    const mono = Monogram({ ins: () => ins });
    host.append(line, mono.id);

    const nameCol = host.column(1);
    host.setGrow(nameCol, 1);
    host.append(line, nameCol);
    const name = host.text(ins.name, 14, INK);
    host.setMaxLines(name, 1);
    host.append(nameCol, name);
    const sym = host.text(ins.symbol, 10, MUTED);
    host.append(nameCol, sym);

    const quoteCol = host.column(1);
    host.setMinSize(quoteCol, 88, 0);
    host.append(line, quoteCol);
    const lastCell = QuoteText({
      symbol: () => item.symbol,
      kind: "last",
      fontSize: 14,
      align: "right",
      binder: () => binder,
    });
    host.append(quoteCol, lastCell.id);
    const pctCell = QuoteText({
      symbol: () => item.symbol,
      kind: "pct",
      fontSize: 12,
      align: "right",
      binder: () => binder,
    });
    host.append(quoteCol, pctCell.id);
    return row;
  }

  return { id: page };
}

function labelOf(m: "hk" | "us" | "cn"): string {
  return m === "hk" ? loc("港股", "HK") : m === "us" ? loc("美股", "US") : loc("A股", "CN");
}

function boardLabel(b: Board): string {
  return b === "gainers" ? loc("涨幅榜", "Gainers") : b === "losers" ? loc("跌幅榜", "Losers") : loc("成交额", "Turnover");
}
