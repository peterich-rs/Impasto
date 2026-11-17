import { onCleanup } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { ScrollArea } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { padded } from "./palette.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const INDICES = [
  "HSI.HK",
  "HSTECH.HK",
  "SPX.US",
  "NDX.US",
  "CSI300.CN",
] as const;

/** Horizontal index strip pinned above the watchlist. One binder consumer. */
export function IndexStrip(props: {
  symbols: () => string[];
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  onClick?: (symbol: string) => void;
}): NodeHandle {
  const row = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 8,
    },
  });
  for (const symbol of INDICES) {
    const card = scene.createElement("view", {
      style: {
        backgroundColor: colorBytes("#FFFFFF"),
        borderRadius: 10,
      },
    });
    scene.patchStyle(card, {
      minWidth: 96,
      minHeight: 56,
    });
    scene.patchStyle(card, {
      outlineWidth: 1,
      outlineColor: colorBytes("#E8E8EC"),
    });
    scene.onClick(card, () => props.onClick?.(symbol));
    scene.append(row, card);
    const ins = props.engine().ins(symbol);
    const name = scene.createElement("text", {
      value: ins.name,
      style: {
        fontSize: 10,
        color: MUTED,
      },
    });
    scene.patchStyle(name, {
      maxLines: 1,
    });
    scene.append(card, padded(name, 8, 8, 0, 8));
    const col = scene.createElement("view", {
      style: {
        gap: 1,
      },
    });
    scene.patchStyle(col, {
      flex: 1,
    });
    scene.append(card, padded(col, 0, 8, 6, 8));
    const lastCell = bindCell(symbol, "last", 15, 600);
    scene.append(col, lastCell);
    const pctCell = bindCell(symbol, "pct", 10);
    scene.append(col, pctCell);
  }
  function bindCell(
    symbol: string,
    kind: "last" | "pct",
    size: number,
    weight?: number,
  ): number {
    const node = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: size,
        color: INK,
      },
    });
    if (weight !== undefined) {
      scene.patchStyle(node, {
        fontWeight: weight,
      });
    }
    const cells =
      kind === "last"
        ? {
            last: node,
            flashNode: node,
          }
        : {
            pct: node,
            flashNode: node,
          };
    onCleanup(props.binder().bind(symbol, cells));
    return node;
  }
  const scroller = ScrollArea({
    axis: "horizontal",
    children: row,
  });
  // `fill` would grow this strip on the page column. Keep it one card tall.
  scene.patchStyle(scroller, {
    flexGrow: 0,
  });
  scene.patchStyle(scroller, {
    minWidth: 0,
    minHeight: 72,
  });
  scene.patchStyle(scroller, {
    maxWidth: 4096,
    maxHeight: 72,
  });
  return scroller;
}
