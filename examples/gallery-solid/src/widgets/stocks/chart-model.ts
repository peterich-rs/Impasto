import type { CanvasRenderingContext2D } from "@impasto/solid-renderer";
import { fmtPrice } from "../../format.ts";
import { marketOpen, minute, type MinuteBar, type Period } from "../../engine/analytics.ts";
import type { Bar } from "../../engine/types.ts";
import { loc } from "../../copy.ts";
import { clock, colors, compact, sign, tone } from "./theme.ts";

const stockTime = (t: number) => clock(marketOpen * minute + t, false);

export function stroke(ctx: CanvasRenderingContext2D, color: string, x0: number, y0: number, x1: number, y1: number): void {
  ctx.strokeStyle = color;
  ctx.beginPath();
  ctx.moveTo(x0, y0);
  ctx.lineTo(x1, y1);
  ctx.stroke();
}

export function dot(ctx: CanvasRenderingContext2D, color: string, x: number, y: number, radius: number): void {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, Math.PI * 2);
  ctx.fill();
}

function average(bars: readonly Bar[], index: number, length: number): number | undefined {
  if (index < length - 1) return undefined;
  let sum = 0;
  for (let offset = 0; offset < length; offset++) sum += bars[index - offset]!.c;
  return sum / length;
}

/** Axis and tooltip copy for one bar. `1D` counts back from the latest bar in `bars`. */
export function barLabel(period: Period, bars: readonly Bar[], bar: Bar): string {
  if (period !== "1D") return stockTime(bar.t);
  if (bar === bars.at(-1)) return loc("今日", "Today");
  const age = bars.length - 1 - bars.indexOf(bar);
  return loc(`${age}天前`, `${age}d ago`);
}

/**
 * Exact `t` inside the half-open window `[first, end)`. The first duplicate wins.
 * A bar that has scrolled or aggregated out of the window is `null`.
 */
export function resolveSelection(time: number | null, bars: readonly Bar[], first: number, end: number): number | null {
  if (time === null) return null;
  for (let index = first; index < end; index++) {
    if (bars[index]?.t === time) return index;
  }
  return null;
}

type Point = { x: number; y: number };
type Align = "left" | "center" | "right";

export type ChartGeometry = {
  width: number;
  height: number;
  plot: number;
  top: number;
  bottom: number;
  volumeBottom: number;
  first: number;
  end: number;
  intraday: boolean;
  previous: number;
  lastTone: string;
  grid: { y: number; label: string }[];
  shadeFrom: number | null;
  prevY: number | null;
  close: Point[];
  averageLine: Point[];
  marker: Point | null;
  candles: { x: number; color: string; high: number; low: number; body: number; bodyY: number; bodyHeight: number; hollow: boolean }[];
  averages: { color: string; points: Point[] }[];
  lastY: number | null;
  volumes: { x: number; y: number; w: number; h: number; color: string }[];
  times: { text: string; align: Align; x: number }[];
  pillY: number;
  pill: string;
  hit(x: number): number | null;
  xAt(index: number): number;
  yAt(price: number): number;
};

export function buildChartGeometry(input: {
  bars: readonly Bar[];
  period: Period;
  startOffset: number;
  slot: number;
  width: number;
  height: number;
  previous: number;
}): ChartGeometry | null {
  const { bars, period, startOffset, slot, width, height, previous } = input;
  if (width < 100 || height < 90 || bars.length === 0) return null;
  const intraday = period === "intraday";
  const plot = width - 52;
  const top = 8;
  const bottom = height - 57;
  const volumeTop = bottom + 10;
  const volumeBottom = height - 18;
  const count = Math.max(1, Math.floor(plot / slot));
  const end = intraday ? bars.length : Math.max(Math.min(bars.length, count), bars.length - startOffset);
  const first = intraday ? 0 : Math.max(0, end - count);
  const visible = bars.slice(first, end);
  const span = Math.max(60 * minute, (bars.at(-1)!.t - bars[0]!.t) * 1.06);
  const xs = visible.map((_, index) => intraday
    ? (visible[index]!.t - bars[0]!.t) / span * plot
    : (index + 0.5) * plot / visible.length);
  const values = visible.map((bar) => bar.c);
  const low = intraday
    ? Math.min(previous, ...values, ...visible.map((bar) => (bar as MinuteBar).avg))
    : Math.min(...visible.map((bar) => bar.l));
  const high = intraday
    ? Math.max(previous, ...values, ...visible.map((bar) => (bar as MinuteBar).avg))
    : Math.max(...visible.map((bar) => bar.h));
  const margin = Math.max(previous * 0.0005, (high - low) * 0.08, 0.01);
  const range = high - low + 2 * margin;
  const yAt = (price: number) => top + (high + margin - price) / range * (bottom - top);
  const xAt = (index: number) => xs[index - first]!;
  const hit = (point: number) => {
    if (point < 0 || point > plot) return null;
    if (!intraday) return first + Math.min(visible.length - 1, Math.floor(point / plot * visible.length));
    let nearest = 0;
    while (nearest + 1 < visible.length && (xs[nearest]! + xs[nearest + 1]!) / 2 <= point) nearest++;
    return nearest;
  };
  const last = visible.at(-1)!;
  const lastIndex = visible.length - 1;
  const lastTone = tone(last.c - previous);
  const grid = [0, 1, 2, 3, 4].map((line) => {
    const axisY = top + line * (bottom - top) / 4;
    const price = high + margin - line * range / 4;
    return { y: axisY, label: intraday ? `${sign((price / previous - 1) * 100)}%` : fmtPrice(price) };
  });
  const close = intraday ? visible.map((bar, index) => ({ x: xs[index]!, y: yAt(bar.c) })) : [];
  const averageLine = intraday ? visible.map((bar, index) => ({ x: xs[index]!, y: yAt((bar as MinuteBar).avg) })) : [];
  const body = Math.max(1, plot / visible.length * 0.63);
  const candles = intraday ? [] : visible.map((bar, index) => {
    const bodyY = Math.min(yAt(bar.o), yAt(bar.c));
    const bodyHeight = Math.max(1, Math.abs(yAt(bar.o) - yAt(bar.c)));
    return {
      x: xs[index]!,
      color: tone(bar.c - bar.o),
      high: yAt(bar.h),
      low: yAt(bar.l),
      body,
      bodyY,
      bodyHeight,
      hollow: bar.c > bar.o && body >= 3 && bodyHeight >= 3,
    };
  });
  const lengths = [5, 10, 20] as const;
  const averageColors = [colors.lime, colors.blue, colors.amber];
  const averages = intraday ? [] : lengths.flatMap((length, slotIndex) => {
    const from = Math.max(0, length - 1 - first);
    if (from >= visible.length) return [];
    const points = [];
    for (let index = from; index < visible.length; index++) {
      points.push({ x: xs[index]!, y: yAt(average(bars, first + index, length)!) });
    }
    return [{ color: averageColors[slotIndex]!, points }];
  });
  const volumeScale = Math.max(1, ...visible.map((bar) => bar.v));
  const volumeWidth = Math.max(1, plot / (intraday ? span / minute : visible.length) * 0.73);
  const volumes = visible.map((bar, index) => {
    const barHeight = bar.v / volumeScale * (volumeBottom - volumeTop);
    return {
      x: Math.max(0, xs[index]! - volumeWidth / 2),
      y: volumeBottom - barHeight,
      w: volumeWidth,
      h: barHeight,
      color: `${tone(bar.c - (index > 0 ? visible[index - 1]!.c : bar.o))}88`,
    };
  });
  const axisTimes = intraday
    ? [stockTime(bars[0]!.t), stockTime(bars[0]!.t + span / 2), stockTime(bars[0]!.t + span)]
    : [barLabel(period, bars, visible[0]!), barLabel(period, bars, visible[Math.floor(lastIndex / 2)]!), barLabel(period, bars, last)];
  const aligns = ["left", "center", "right"] as const;
  return {
    width,
    height,
    plot,
    top,
    bottom,
    volumeBottom,
    first,
    end,
    intraday,
    previous,
    lastTone,
    grid,
    shadeFrom: intraday ? xs[lastIndex]! + 4 : null,
    prevY: intraday ? yAt(previous) : null,
    close,
    averageLine,
    marker: intraday ? { x: xs[lastIndex]!, y: yAt(last.c) } : null,
    candles,
    averages,
    lastY: intraday ? null : yAt(last.c),
    volumes,
    times: axisTimes.map((text, index) => ({ text, align: aligns[index]!, x: index * plot / 2 })),
    pillY: Math.max(top + 8, Math.min(bottom - 8, yAt(last.c))),
    pill: fmtPrice(last.c),
    hit,
    xAt,
    yAt,
  };
}

function trace(ctx: CanvasRenderingContext2D, points: readonly Point[]): void {
  const first = points[0];
  if (!first) return;
  ctx.beginPath();
  const coordinates = new Float64Array(points.length * 2);
  for (let index = 0; index < points.length; index++) {
    coordinates[index * 2] = points[index]!.x;
    coordinates[index * 2 + 1] = points[index]!.y;
  }
  ctx.polyline(coordinates);
}

function shading(ctx: CanvasRenderingContext2D, width: number, from: number, top: number, height: number): void {
  if (from >= width) return;
  ctx.save();
  ctx.beginPath();
  ctx.rect(from, top, width - from, height);
  ctx.clip();
  ctx.strokeStyle = colors.line;
  ctx.beginPath();
  const start = from - height;
  const segments = new Float64Array(Math.ceil((width - start) / 7) * 4);
  for (let index = 0; index < segments.length; index += 4) {
    const x = start + index * 7 / 4;
    segments.set([x, top + height, x + height, top], index);
  }
  ctx.lineSegments(segments);
  ctx.stroke();
  ctx.restore();
}

/** Main plot only. The crosshair is an overlay and is not read here. */
export function paintChart(ctx: CanvasRenderingContext2D, geo: ChartGeometry): void {
  for (const line of geo.grid) {
    stroke(ctx, colors.line, 0, line.y, geo.plot, line.y);
  }
  if (geo.shadeFrom !== null) {
    shading(ctx, geo.plot, geo.shadeFrom, geo.top, geo.bottom - geo.top);
    ctx.setLineDash([3, 4]);
    stroke(ctx, colors.faint, 0, geo.prevY!, geo.plot, geo.prevY!);
    ctx.setLineDash([]);
    trace(ctx, geo.close);
    ctx.lineTo(geo.close.at(-1)!.x, geo.bottom);
    ctx.lineTo(geo.close[0]!.x, geo.bottom);
    ctx.closePath();
    const gradient = ctx.createLinearGradient(0, geo.top, 0, geo.bottom);
    gradient.addColorStop(0, `${geo.lastTone}43`);
    gradient.addColorStop(1, `${geo.lastTone}00`);
    ctx.fillStyle = gradient;
    ctx.fill();
    trace(ctx, geo.close);
    ctx.strokeStyle = geo.lastTone;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.stroke();
    trace(ctx, geo.averageLine);
    ctx.strokeStyle = colors.amber;
    ctx.lineWidth = 1;
    ctx.stroke();
    dot(ctx, `${geo.lastTone}30`, geo.marker!.x, geo.marker!.y, 8);
    dot(ctx, geo.lastTone, geo.marker!.x, geo.marker!.y, 3);
  } else {
    for (const candle of geo.candles) {
      stroke(ctx, candle.color, candle.x, candle.high, candle.x, candle.low);
      const left = candle.x - candle.body / 2;
      if (candle.hollow) {
        ctx.fillStyle = colors.bg;
        ctx.fillRect(left, candle.bodyY, candle.body, candle.bodyHeight);
        ctx.strokeStyle = candle.color;
        ctx.strokeRect(left, candle.bodyY, candle.body, candle.bodyHeight);
      } else {
        ctx.fillStyle = candle.color;
        ctx.fillRect(left, candle.bodyY, candle.body, candle.bodyHeight);
      }
    }
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, geo.top, geo.plot, geo.bottom - geo.top);
    ctx.clip();
    for (const line of geo.averages) {
      trace(ctx, line.points);
      ctx.strokeStyle = line.color;
      ctx.stroke();
    }
    ctx.restore();
    ctx.setLineDash([3, 3]);
    stroke(ctx, `${geo.lastTone}90`, 0, geo.lastY!, geo.plot, geo.lastY!);
    ctx.setLineDash([]);
  }
  for (let first = 0; first < geo.volumes.length;) {
    const color = geo.volumes[first]!.color;
    let end = first + 1;
    while (end < geo.volumes.length && geo.volumes[end]!.color === color) end++;
    ctx.fillStyle = color;
    if (end === first + 1) {
      const { x, y, w, h } = geo.volumes[first]!;
      ctx.fillRect(x, y, w, h);
    } else {
      const rects = new Float64Array((end - first) * 4);
      for (let index = first; index < end; index++) {
        const { x, y, w, h } = geo.volumes[index]!;
        rects.set([x, y, w, h], (index - first) * 4);
      }
      ctx.fillRects(rects);
    }
    first = end;
  }
  stroke(ctx, colors.line, 0, geo.volumeBottom, geo.plot, geo.volumeBottom);
  ctx.font = "10px Sans";
  ctx.textBaseline = "middle";
  ctx.textAlign = "right";
  ctx.fillStyle = colors.faint;
  for (const line of geo.grid) {
    ctx.fillText(line.label, geo.width, line.y, 49);
  }
  for (const label of geo.times) {
    ctx.textAlign = label.align;
    ctx.fillText(label.text, label.x, geo.height - 7, geo.plot / 3);
  }
  ctx.fillStyle = geo.lastTone;
  ctx.beginPath();
  ctx.roundRect(geo.plot + 3, geo.pillY - 8, 49, 16, 5);
  ctx.fill();
  ctx.fillStyle = colors.bg;
  ctx.textAlign = "center";
  ctx.fillText(geo.pill, geo.plot + 27, geo.pillY, 45);
}
