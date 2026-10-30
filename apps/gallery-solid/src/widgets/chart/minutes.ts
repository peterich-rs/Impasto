import { onCleanup, createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import type { QuoteBinder } from "../../state/binder.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import { downsampleTicks } from "../../engine/universe.ts";
import { toneFor, type Updown } from "../../state/settings.ts";

const MUTED = colorBytes("#8A8D91");
const LINE = colorBytes("#C9CBD1");
const COLS = 80;
const PLOT_W = 320;

/**
 * Intraday chart: 80 aggregated columns from the tick ring, refreshed on the
 * binder's ~1Hz spark queue. The previous close runs as a dashed baseline.
 */
export function MinutesChart(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
  h: number;
  onExtents?: (lo: number, hi: number) => void;
}): NodeHandle {
  const h = props.h;
  const root = host.stack();
  host.setMinSize(root, 0, h);

  const base = host.decorated(LINE, 0);
  host.setMinSize(base, 0, 1);
  host.setPosition(base, true);
  host.append(root, base);

  const cols: number[] = [];
  for (let i = 0; i < COLS; i++) {
    const bar = host.decorated(MUTED, 0);
    host.setMinSize(bar, 2, 2);
    host.setPosition(bar, true);
    host.append(root, bar);
    cols.push(bar);
  }

  createTrackedEffect(() => {
    const symbol = props.symbol();
    const engine = props.engine();
    const stop = props.binder().bindSpark(symbol, () => {
      const q = engine.quote(symbol);
      const ring = engine.ring(symbol);
      const series = downsampleTicks(ring, COLS);
      if (series.length === 0) {
        return;
      }
      let lo = Math.min(q.low, q.prevClose);
      let hi = Math.max(q.high, q.prevClose);
      const span = hi - lo || q.prevClose * 0.004 || 1;
      lo -= span * 0.08;
      hi += span * 0.08;
      props.onExtents?.(lo, hi);
      const tone = toneFor(q.last, q.prevClose, props.updown());
      const color =
        tone === "up"
          ? colorBytes("#EB3E3E")
          : tone === "down"
            ? colorBytes("#22B05F")
            : MUTED;
      const baseY = h * (1 - (q.prevClose - lo) / (hi - lo));
      host.setInset(base, 0, Math.round(baseY), 0, 0);
      host.setMinSize(base, 0, 1);
      for (let i = 0; i < cols.length; i++) {
        const p = series[Math.min(i, series.length - 1)]!;
        const y = h * (1 - (p - lo) / (hi - lo));
        const top = Math.min(y, baseY);
        const bottom = Math.max(y, baseY);
        host.setInset(cols[i]!, 0, Math.round(top), 0, Math.round((i * PLOT_W) / (COLS - 1)));
        host.setMinSize(cols[i]!, 2, Math.max(2, Math.round(bottom - top)));
        host.setColor(cols[i]!, color);
      }
    });
    onCleanup(stop);
  });

  return { id: root };
}
