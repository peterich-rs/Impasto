import { onCleanup, createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { downsampleTicks } from "../engine/universe.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { toneFor, type Updown } from "../state/settings.ts";

const COLS = 16;
const BAR_W = 2;
const GAP = 1;
/** Height in scene units each column is scaled against. */
const UNIT_H = 10;
const UP: [number, number, number, number] = [235, 62, 62, 255];
const DOWN: [number, number, number, number] = [34, 176, 95, 255];
const FLAT: [number, number, number, number] = [138, 141, 145, 255];

/**
 * 16-column sparkline. Each column is a unit-height `decorated` bar inside a
 * `transform`. Height is `setScaleXY(1, s)` on the wrap — paint-only, no
 * layout. Scale is around the wrap origin (top-left), so a translate keeps
 * the bar sitting on the box floor. Refresh rides the binder's spark queue,
 * not the per-frame price path.
 */
export function Sparkline(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
  w?: number;
  h?: number;
}): NodeHandle {
  const w = props.w ?? 54;
  const h = props.h ?? 26;
  const slot = BAR_W + GAP;
  const total = COLS * slot - GAP;

  const root = host.stack();
  const cols: { wrap: number; bar: number }[] = [];
  for (let i = 0; i < COLS; i++) {
    const wrap = host.transform();
    host.setPosition(wrap, true);
    host.setInset(wrap, 0, 0, 0, Math.round((i * total) / (COLS - 1)));
    const bar = host.decorated(FLAT, 1);
    host.setMinSize(bar, BAR_W, UNIT_H);
    host.append(wrap, bar);
    host.append(root, wrap);
    cols.push({ wrap, bar });
  }
  host.setMinSize(root, w, h);

  createTrackedEffect(() => {
    const symbol = props.symbol();
    const engine = props.engine();
    const stop = props.binder().bindSpark(symbol, () => {
      const ring = engine.ring(symbol);
      const series = downsampleTicks(ring, COLS);
      if (series.length === 0) {
        return;
      }
      let lo = Infinity;
      let hi = -Infinity;
      for (const p of series) {
        if (p < lo) lo = p;
        if (p > hi) hi = p;
      }
      const span = hi - lo || hi * 0.001 || 1;
      const last = series[series.length - 1]!;
      const tone = toneFor(last, ring.at(0).price, props.updown());
      const color = tone === "up" ? UP : tone === "down" ? DOWN : FLAT;
      for (const [i, col] of cols.entries()) {
        const p = series[Math.min(i, series.length - 1)]!;
        // Map into [0.15, 1] of the box height; scale from the unit bar.
        const s = Math.max(1e-3, (0.15 + (0.85 * (p - lo)) / span) * (h / UNIT_H));
        host.setTransform(col.wrap, 0, h - UNIT_H * s, 0, 1);
        host.setScaleXY(col.wrap, 1, s);
        host.setColor(col.bar, color);
      }
    });
    onCleanup(stop);
  });

  return { id: root };
}
