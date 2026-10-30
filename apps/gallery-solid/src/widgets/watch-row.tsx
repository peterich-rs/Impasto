import { host, type NodeHandle } from "@impasto/solid-renderer";
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

export type RowItem = { symbol: string; firstOfGroup?: string };

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

  const region = host.pointerRegion();
  host.onClick(region, () => props.onClick?.(symbol()));

  const pad = host.padding(0);
  host.setPaddingEdges(pad, 10, 16, 10, 16);
  host.append(region, pad);

  const line = host.row(10);
  host.setMinSize(line, 0, 56);
  host.append(pad, line);

  const mono = Monogram({ ins });
  host.append(line, mono.id);

  const nameCol = host.column(2);
  host.setMinSize(nameCol, 148, 0);
  host.setGrow(nameCol, 1);
  host.append(line, nameCol);
  const title = host.text(ins().name, 15, INK);
  host.setFontWeight(title, 600);
  host.setMaxLines(title, 1);
  host.append(nameCol, title);
  const sub = host.text(`${ins().symbol} · ${labelMarket(ins().market)}`, 11, MUTED);
  host.append(nameCol, sub);

  const spark = Sparkline({
    symbol,
    engine: props.engine,
    binder: props.binder,
    updown: props.updown,
  });
  host.append(line, spark.id);

  const numCol = host.column(2);
  host.setMinSize(numCol, 86, 0);
  host.setCross(line, "end");
  host.append(line, numCol);
  const lastCell = QuoteText({ symbol, kind: "last", fontSize: 15, binder: props.binder });
  host.setFontWeight(lastCell.id, 600);
  host.append(numCol, lastCell.id);
  const pctCell = QuoteText({ symbol, kind: "pct", fontSize: 11, binder: props.binder });
  host.append(numCol, pctCell.id);

  void GAIN;
  void LOSS;
  return { id: region };
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
