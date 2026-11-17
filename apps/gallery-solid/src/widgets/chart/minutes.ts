import { onCleanup, createTrackedEffect } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import type { QuoteBinder } from "../../state/binder.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import { downsampleTicks } from "../../engine/universe.ts";
import { toneFor, type Updown } from "../../state/settings.ts";
const MUTED = colorBytes("#8A8D91");
const LINE = colorBytes("#C9CBD1");
const COLS = 80;

/**
 * Intraday chart: 80 aggregated columns from the tick ring, refreshed on the
 * binder's ~1Hz spark queue. The previous close runs as a dashed baseline.
 * Column spacing uses `viewW()` from the parent box.
 */
export function MinutesChart(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
  h: number;
  viewW: () => number;
  onExtents?: (lo: number, hi: number) => void;
}): NodeHandle {
  const h = props.h;
  const root = scene.stack();
  scene.patchStyle(root, {
    minWidth: 0,
    minHeight: h,
  });
  const base = scene.createElement("view", {
    style: {
      backgroundColor: LINE,
      borderRadius: 0,
    },
  });
  scene.patchStyle(base, {
    minWidth: 0,
    minHeight: 1,
  });
  scene.patchStyle(base, {
    position: "absolute",
  });
  scene.append(root, base);
  const cols: number[] = [];
  for (let i = 0; i < COLS; i++) {
    const bar = scene.createElement("view", {
      style: {
        backgroundColor: MUTED,
        borderRadius: 0,
      },
    });
    scene.patchStyle(bar, {
      minWidth: 2,
      minHeight: 2,
    });
    scene.patchStyle(bar, {
      position: "absolute",
    });
    scene.append(root, bar);
    cols.push(bar);
  }
  createTrackedEffect(() => {
    const symbol = props.symbol();
    const engine = props.engine();
    props.viewW();
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
      const plotW = Math.max(2, props.viewW());
      scene.patchStyle(root, {
        minWidth: plotW,
        minHeight: h,
      });
      const barW = Math.max(1, Math.floor(plotW / COLS) - 1);
      const baseY = h * (1 - (q.prevClose - lo) / (hi - lo));
      scene.patchStyle(base, {
        top: 0,
        right: Math.round(baseY),
        bottom: 0,
        left: 0,
      });
      scene.patchStyle(base, {
        minWidth: 0,
        minHeight: 1,
      });
      for (let i = 0; i < cols.length; i++) {
        const p = series[Math.min(i, series.length - 1)]!;
        const y = h * (1 - (p - lo) / (hi - lo));
        const top = Math.min(y, baseY);
        const bottom = Math.max(y, baseY);
        scene.patchStyle(cols[i]!, {
          top: 0,
          right: Math.round(top),
          bottom: 0,
          left: Math.round((i * plotW) / (COLS - 1)),
        });
        scene.patchStyle(cols[i]!, {
          minWidth: barW,
          minHeight: Math.max(2, Math.round(bottom - top)),
        });
        scene.patchStyle(cols[i]!, {
          backgroundColor: color,
        });
      }
    });
    onCleanup(stop);
  });
  return root;
}
