import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { Monogram } from "./monogram.tsx";
import { QuoteText } from "./quote-text.tsx";
import { Sparkline } from "./sparkline.tsx";
import type { Updown } from "../state/settings.ts";
const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const GAIN = colorBytes("#EB3E3E");
const LOSS = colorBytes("#22B05F");
export type RowItem = {
  symbol: string;
  firstOfGroup?: string;
};

/**
 * One watchlist / market row. Left: monogram + names. Right: sparkline and
 * the two binder-driven cells. Fixed widths keep the layout stable while
 * prices change every frame.
 */
export function WatchRow(props: {
  item: () => RowItem;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
  onClick?: (symbol: string) => void;
}): NodeHandle {
  const symbol = () => props.item().symbol;
  const ins = () => props.engine().ins(symbol());
  const region = scene.createElement("view", {
    style: {},
  });
  scene.onClick(region, () => props.onClick?.(symbol()));
  const pad = scene.createElement("view", {
    style: {
      padding: 0,
    },
  });
  scene.patchStyle(pad, {
    paddingTop: 10,
    paddingRight: 16,
    paddingBottom: 10,
    paddingLeft: 16,
  });
  scene.patchStyle(pad, {
    flex: 1,
  });
  scene.append(region, pad);
  const line = scene.createElement("view", {
    style: {
      flexDirection: "row",
      gap: 10,
    },
  });
  scene.patchStyle(line, {
    minWidth: 0,
    minHeight: 56,
  });
  scene.patchStyle(line, {
    alignItems: "center",
  });
  scene.append(pad, line);
  const mono = Monogram({
    ins,
  });
  scene.append(line, mono);
  const nameCol = scene.createElement("view", {
    style: {
      gap: 2,
    },
  });
  scene.patchStyle(nameCol, {
    minWidth: 0,
    minHeight: 0,
  });
  scene.patchStyle(nameCol, {
    maxWidth: 168,
    maxHeight: 80,
  });
  scene.patchStyle(nameCol, {
    flex: 1,
  });
  scene.append(line, nameCol);
  const title = scene.createElement("text", {
    value: ins().name,
    style: {
      fontSize: 15,
      color: INK,
    },
  });
  scene.patchStyle(title, {
    fontWeight: 600,
  });
  scene.patchStyle(title, {
    maxLines: 1,
  });
  scene.patchStyle(title, {
    textOverflow: "ellipsis",
  });
  scene.patchStyle(title, {
    ellipsis: "…",
  });
  scene.append(nameCol, title);
  const sub = scene.createElement("text", {
    value: `${ins().symbol} · ${labelMarket(ins().market)}`,
    style: {
      fontSize: 11,
      color: MUTED,
    },
  });
  scene.append(nameCol, sub);
  const spark = Sparkline({
    symbol,
    engine: props.engine,
    binder: props.binder,
    updown: props.updown,
    w: 56,
    h: 22,
  });
  scene.append(line, spark);
  const numCol = scene.createElement("view", {
    style: {
      gap: 2,
    },
  });
  scene.append(line, numCol);
  const lastCell = QuoteText({
    symbol,
    kind: "last",
    fontSize: 15,
    weight: 600,
    align: "right",
    binder: props.binder,
  });
  scene.append(numCol, lastCell);
  const pctCell = QuoteText({
    symbol,
    kind: "pct",
    fontSize: 11,
    align: "right",
    binder: props.binder,
  });
  scene.append(numCol, pctCell);
  void GAIN;
  void LOSS;
  return region;
}
function labelMarket(market: string): string {
  if (market === "hk") {
    return "HK";
  }
  if (market === "us") {
    return "US";
  }
  return "SH/SZ";
}
