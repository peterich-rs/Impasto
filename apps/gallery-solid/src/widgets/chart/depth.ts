import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import { round2 } from "../../engine/universe.ts";
import { toneFor, type Updown } from "../../state/settings.ts";
import { fmtVolumeCN } from "../../format.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const UP_BG = colorBytes("#FDECEC");
const DOWN_BG = colorBytes("#E8F7EF");

/**
 * Ten-level depth ladder. Prices are deterministic around the last tick; the
 * volume bars widen toward the middle of the book. Re-annotated on each spark
 * refresh (~1Hz), never per frame.
 */
export function DepthLadder(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  updown: () => Updown;
  onPick?: (price: number) => void;
}): NodeHandle {
  const root = host.column(0);

  const askRows: { price: number; vol: number; bar: number }[] = [];
  const bidRows: { price: number; vol: number; bar: number }[] = [];

  // asks (reversed so ask5 is on top, ask1 nearest the spread)
  for (let level = 5; level >= 1; level--) {
    askRows.push(buildRow(level, "ask"));
  }
  const spread = host.column(2);
  host.append(root, spread);
  const spreadLabel = host.text("—", 13, INK);
  host.setFontWeight(spreadLabel, 600);
  host.append(spread, spreadLabel);
  const spreadHint = host.text("", 10, MUTED);
  host.append(spread, spreadHint);
  for (let level = 1; level <= 5; level++) {
    bidRows.push(buildRow(level, "bid"));
  }

  function buildRow(level: number, side: "ask" | "bid") {
    const row = host.pointerRegion();
    host.setMinSize(row, 0, 22);
    host.append(root, row);
    const bar = host.decorated(side === "ask" ? UP_BG : DOWN_BG, 2);
    host.setPosition(bar, true);
    host.setInset(bar, 0, 0, 0, 0);
    host.append(row, bar);
    const price = host.text("—", 12, INK);
    const vol = host.text("—", 11, MUTED);
    const pad = host.padding(0);
    host.setPaddingEdges(pad, 6, 6, 4, 6);
    host.append(row, pad);
    const line = host.row(4);
    host.append(pad, line);
    host.append(line, price);
    const spacer = host.sized(0, 0);
    host.setGrow(spacer, 1);
    host.append(line, spacer);
    host.append(line, vol);
    host.onClick(row, () => {
      const q = props.engine().quote(props.symbol());
      const step = q.last * 0.001;
      const at = round2(q.last + (side === "ask" ? 1 : -1) * step * level);
      props.onPick?.(at);
    });
    return { price, vol, bar, level, side: side as "ask" | "bid" };
  }

  function refresh(): void {
    const engine = props.engine();
    const symbol = props.symbol();
    const q = engine.quote(symbol);
    const rand = pseudoFor(symbol, q.revision);
    const step = Math.max(0.01, q.last * 0.001);
    const maxBar = 110;
    const spreadNow = round2(step * 2);
    host.setText(spreadLabel, fmtSpread(q.last));
    host.setText(spreadHint, `价差 ${spreadNow.toFixed(2)}`);
    host.setColor(spreadLabel, toneColor(q.last, q.prevClose));
    for (const row of [...askRows, ...bidRows]) {
      const dir = row.side === "ask" ? 1 : -1;
      const price = round2(q.last + dir * step * row.level);
      const vol = Math.floor(80 + rand() * 900) * (engine.ins(symbol).lot === 1 ? 1 : 100);
      host.setText(row.price, price.toFixed(2));
      host.setText(row.vol, fmtVolumeCN(vol));
      host.setColor(row.price, toneColor(q.last, q.prevClose));
      const w = Math.max(18, Math.round(maxBar * (0.2 + 0.8 * rand())));
      host.setMinSize(row.bar, w, 16);
    }
  }

  function toneColor(last: number, prevClose: number): [number, number, number, number] {
    const tone = toneFor(last, prevClose, props.updown());
    if (tone === "up") {
      return colorBytes("#EB3E3E");
    }
    if (tone === "down") {
      return colorBytes("#22B05F");
    }
    return MUTED;
  }

  refresh();
  return { id: root, refresh };
}

function fmtSpread(last: number): string {
  return last.toFixed(2);
}

function pseudoFor(symbol: string, revision: number): () => number {
  let x = (revision * 2654435761) ^ hashLite(symbol);
  return () => {
    x = (Math.imul(x, 48271) + 11) >>> 0;
    return (x % 10000) / 10000;
  };
}

function hashLite(value: string): number {
  let h = 5381;
  for (let i = 0; i < value.length; i++) {
    h = (Math.imul(h, 33) ^ value.charCodeAt(i)) >>> 0;
  }
  return h;
}
