import { host } from "@impasto/solid-renderer";
import type { Bar } from "../../engine/types.ts";
import { colorBytes } from "../../colors.ts";

export const OVERSCAN = 10;
const UP = colorBytes("#EB3E3E");
const DOWN = colorBytes("#22B05F");

export type ChartGeom = {
  slotW: number;
  plotH: number;
  volH: number;
  bars: readonly Bar[];
  lo: number;
  hi: number;
  maxVol: number;
};

export function geomFor(bars: readonly Bar[], slotW: number, plotH: number, volH: number, first: number, last: number): ChartGeom {
  let lo = Infinity;
  let hi = -Infinity;
  let maxVol = 0;
  for (let i = first; i <= last && i < bars.length; i++) {
    const b = bars[i]!;
    if (b.l < lo) lo = b.l;
    if (b.h > hi) hi = b.h;
    if (b.v > maxVol) maxVol = b.v;
  }
  if (!Number.isFinite(lo) || !Number.isFinite(hi) || hi <= lo) {
    const ref = bars[bars.length - 1]?.c ?? 1;
    lo = ref * 0.99;
    hi = ref * 1.01;
  }
  const pad = (hi - lo) * 0.06;
  return { slotW, plotH, volH, bars, lo: lo - pad, hi: hi + pad, maxVol: maxVol || 1 };
}

export function visibleRange(scrollX: number, viewW: number, geom: ChartGeom): [number, number] {
  const total = geom.bars.length;
  const first = Math.max(0, Math.floor(scrollX / geom.slotW) - OVERSCAN);
  const last = Math.min(total - 1, Math.ceil((scrollX + viewW) / geom.slotW) + OVERSCAN);
  return [first, Math.max(first, last)];
}

export function yOf(price: number, geom: ChartGeom): number {
  const span = geom.hi - geom.lo || 1;
  return geom.plotH * (1 - (price - geom.lo) / span);
}

export type CandleRects = {
  cx: number;
  wickY: number;
  wickH: number;
  bodyY: number;
  bodyH: number;
  bodyW: number;
  up: boolean;
};

export function candleRects(bar: Bar, index: number, geom: ChartGeom): CandleRects {
  const cx = index * geom.slotW + geom.slotW / 2;
  const yo = yOf(bar.o, geom);
  const yc = yOf(bar.c, geom);
  const yh = yOf(bar.h, geom);
  const yl = yOf(bar.l, geom);
  return {
    cx,
    wickY: yh,
    wickH: Math.max(1, yl - yh),
    bodyY: Math.min(yo, yc),
    bodyH: Math.max(1.5, Math.abs(yc - yo)),
    bodyW: Math.max(1.5, geom.slotW - 2),
    up: bar.c >= bar.o,
  };
}

export function candleColor(up: boolean): [number, number, number, number] {
  return up ? UP : DOWN;
}
