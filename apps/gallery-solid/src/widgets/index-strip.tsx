import { onCleanup } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import { padded } from "./palette.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");

/** Horizontal index strip pinned above the watchlist. One binder consumer. */
export function IndexStrip(props: {
  symbols: () => string[];
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  onClick?: (symbol: string) => void;
}): NodeHandle {
  const strip = host.row(8);

  createStrip();

  function createStrip(): void {
    for (const symbol of ["HSI.HK", "HSTECH.HK", "SPX.US", "NDX.US", "CSI300.CN"]) {
      const card = host.decorated(colorBytes("#FFFFFF"), 10);
      host.setMinSize(card, 96, 56);
      host.setStroke(card, 1, colorBytes("#E8E8EC"));
      host.onClick(card, () => props.onClick?.(symbol));
      host.append(strip, card);

      const ins = props.engine().ins(symbol);
      const name = host.text(ins.name, 10, MUTED);
      host.setMaxLines(name, 1);
      host.append(card, padded(name, 8, 8, 0, 8));

      const col = host.column(1);
      host.setGrow(col, 1);
      host.append(card, padded(col, 0, 8, 6, 8));
      const lastCell = props.binder() && bindCell(symbol, "last", 15, 600);
      host.append(col, lastCell);
      const pctCell = bindCell(symbol, "pct", 10);
      host.append(col, pctCell);
    }
  }

  function bindCell(symbol: string, kind: "last" | "pct", size: number, weight?: number): number {
    // Inline a minimal binder cell: a text node bound on mount. The strip is
    // created once, so a plain closure suffices (no recycle here).
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

  return { id: strip };
}
