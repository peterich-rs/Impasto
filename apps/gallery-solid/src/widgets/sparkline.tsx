import { onCleanup, createTrackedEffect } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { downsampleTicks } from "../engine/universe.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { toneFor, type Updown } from "../state/settings.ts";
const COLS = 16;
const BAR_W = 2;
const GAP = 2;
const UP: [number, number, number, number] = [235, 62, 62, 255];
const DOWN: [number, number, number, number] = [34, 176, 95, 255];
const FLAT: [number, number, number, number] = [138, 141, 145, 255];

/**
 * 16-column sparkline. Each bar is an absolute view with an explicit height.
 * Its top inset keeps the bottom on the floor. Refresh uses the binder's queue.
 */
export function Sparkline(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
  w?: number;
  h?: number;
}): NodeHandle {
  const w = props.w ?? COLS * (BAR_W + GAP) - GAP;
  const h = props.h ?? 26;
  const slot = BAR_W + GAP;
  const root = scene.stack();
  scene.patchStyle(root, {
    minWidth: w,
    minHeight: h,
  });
  scene.patchStyle(root, {
    maxWidth: w,
    maxHeight: h,
  });
  const cols: number[] = [];
  for (let i = 0; i < COLS; i++) {
    const bar = scene.createElement("view", {
      style: {
        backgroundColor: FLAT,
        borderRadius: 0,
      },
    });
    scene.patchStyle(bar, {
      position: "absolute",
    });
    scene.patchStyle(bar, {
      width: BAR_W,
      height: 2,
    });
    scene.patchStyle(bar, {
      top: h - 2,
      right: 0,
      bottom: 0,
      left: i * slot,
    });
    scene.append(root, bar);
    cols.push(bar);
  }
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
      for (const [i, bar] of cols.entries()) {
        const p = series[Math.min(i, series.length - 1)]!;
        const bh = Math.max(
          2,
          Math.round((0.15 + (0.85 * (p - lo)) / span) * h),
        );
        scene.patchStyle(bar, {
          width: BAR_W,
          height: bh,
        });
        scene.patchStyle(bar, {
          top: h - bh,
          right: 0,
          bottom: 0,
          left: i * slot,
        });
        scene.patchStyle(bar, {
          backgroundColor: color,
        });
      }
    });
    onCleanup(stop);
  });
  return root;
}
