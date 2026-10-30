import { createRenderEffect, createMemo, createSignal } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import { clearChildren, padded } from "../widgets/palette.ts";
import { useQuotes } from "../state/quotes-provider.tsx";
import { settingsSig } from "../state/settings.ts";
import type { Router } from "@impasto/solid-router";
import { QuoteText } from "../widgets/quote-text.tsx";
import { Monogram } from "../widgets/monogram.tsx";

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
  void settingsSig;

  const page = host.column(10);
  host.setGrow(page, 1);

  const title = host.text("市场", 26, INK);
  host.setFontWeight(title, 700);
  const titlePad = host.padding(0);
  host.setPaddingEdges(titlePad, 12, 16, 0, 16);
  host.append(titlePad, title);
  host.append(page, titlePad);

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
    host.onClick(cell, () => setMarket(m));
    host.append(pills, cell);
    const pill = host.decorated(PANEL, 999);
    host.setStroke(pill, 1, HAIR);
    host.setMinSize(pill, 0, 30);
    host.append(cell, pill);
    const label = host.text(labelOf(m), 12, MUTED);
    host.append(pill, padded(label, 12, 14, 6, 14));
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
    host.append(cell, inner);
    const label = host.text(boardLabel(b), 13, MUTED);
    const align = host.align("center");
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
    const all = engine
      .instruments()
      .filter((ins) => ins.market === market() && !ins.index);
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

  const board = host.column(0);
  host.append(panel, board);

  createRenderEffect(
    () => items(),
    (list) => {
      clearChildren(board);
      for (const [i, item] of list.entries()) {
        host.append(board, buildBoardRow(item, i));
      }
    },
  );

  function buildBoardRow(item: BoardItem, index: number): number {
    const ins = engine.ins(item.symbol);
    const row = host.pointerRegion();
    host.onClick(row, () => props.router.push(`/stock/${item.symbol}`));
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 10, 16, 10, 16);
    host.append(row, pad);
    const line = host.row(8);
    host.setMinSize(line, 0, 44);
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

    const lastCell = QuoteText({ symbol: () => item.symbol, kind: "last", fontSize: 14, binder: () => binder });
    host.append(line, lastCell.id);
    const pctCell = QuoteText({ symbol: () => item.symbol, kind: "pct", fontSize: 12, binder: () => binder });
    host.append(line, pctCell.id);
    return row;
  }

  return { id: page };
}

function labelOf(m: "hk" | "us" | "cn"): string {
  return m === "hk" ? "港股" : m === "us" ? "美股" : "A股";
}

function boardLabel(b: Board): string {
  return b === "gainers" ? "涨幅榜" : b === "losers" ? "跌幅榜" : "成交额";
}
