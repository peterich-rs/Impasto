import { onCleanup } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { ScrollArea } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { padded } from "./palette.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const INDICES = ["HSI.HK", "HSTECH.HK", "SPX.US", "NDX.US", "CSI300.CN"] as const;

/** Horizontal index strip pinned above the watchlist. One binder consumer. */
export function IndexStrip(props: {
  symbols: () => string[];
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  onClick?: (symbol: string) => void;
}): NodeHandle {
  const row = host.row(8);
  for (const symbol of INDICES) {
    const card = host.decorated(colorBytes("#FFFFFF"), 10);
    host.setMinSize(card, 96, 56);
    host.setStroke(card, 1, colorBytes("#E8E8EC"));
    host.onClick(card, () => props.onClick?.(symbol));
    host.append(row, card);

    const ins = props.engine().ins(symbol);
    const name = host.text(ins.name, 10, MUTED);
    host.setMaxLines(name, 1);
    host.append(card, padded(name, 8, 8, 0, 8));

    const col = host.column(1);
    host.setGrow(col, 1);
    host.append(card, padded(col, 0, 8, 6, 8));
    const lastCell = bindCell(symbol, "last", 15, 600);
    host.append(col, lastCell);
    const pctCell = bindCell(symbol, "pct", 10);
    host.append(col, pctCell);
  }

  function bindCell(symbol: string, kind: "last" | "pct", size: number, weight?: number): number {
    const node = host.text("—", size, INK);
    if (weight !== undefined) {
      host.setFontWeight(node, weight);
    }
    const cells =
      kind === "last"
        ? { last: node, flashNode: node }
        : { pct: node, flashNode: node };
    onCleanup(props.binder().bind(symbol, cells));
    return node;
  }

  const scroller = ScrollArea({
    axis: "horizontal",
    children: { id: row },
  });
  // `fill` would grow this strip on the page column. Keep it one card tall.
  host.setGrow(scroller.id, 0);
  host.setMinSize(scroller.id, 0, 72);
  host.setMaxSize(scroller.id, 4096, 72);
  return scroller;
}
