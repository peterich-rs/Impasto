import { onCleanup, createTrackedEffect } from "solid-js";
import type { NodeHandle } from "@impasto/solid-renderer";
import { Canvas, type CanvasController } from "@impasto/solid-ui";
import { downsampleTicks } from "../engine/universe.ts";
import type { QuoteBinder } from "../state/binder.ts";
import type { QuoteEngine } from "../engine/engine.ts";
import { toneFor, type Updown } from "../state/settings.ts";

const UP = "#EB3E3E";
const DOWN = "#22B05F";
const FLAT = "#8A8D91";

/** Latest ticks as one stroke. The binder invalidates; symbol and tone stay reactive. */
export function Sparkline(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  binder: () => QuoteBinder;
  updown: () => Updown;
}): NodeHandle {
  let picture: CanvasController | null = null;
  createTrackedEffect(() => {
    const symbol = props.symbol();
    const stop = props.binder().bindSpark(symbol, () => {
      picture?.invalidate();
    });
    onCleanup(stop);
  });
  return Canvas({
    style: { width: 104, height: 40 },
    accessibilityLabel: "最近行情走势",
    controller: (next) => {
      picture = next;
    },
    draw: (ctx, size) => {
      const symbol = props.symbol();
      const engine = props.engine();
      const updown = props.updown();
      const ring = engine.ring(symbol);
      const series = downsampleTicks(ring, 16);
      if (series.length === 0) return;
      let lo = Infinity;
      let hi = -Infinity;
      for (const price of series) {
        if (price < lo) lo = price;
        if (price > hi) hi = price;
      }
      const span = hi - lo || hi * 0.001 || 1;
      const last = series[series.length - 1] ?? lo;
      const tone = toneFor(last, ring.at(0).price, updown);
      ctx.strokeStyle = tone === "up" ? UP : tone === "down" ? DOWN : FLAT;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.beginPath();
      for (let index = 0; index < series.length; index += 1) {
        const price = series[index] ?? lo;
        const x = series.length === 1 ? 0 : (index * size.width) / (series.length - 1);
        const y = size.height - 2 - ((price - lo) / span) * (size.height - 4);
        if (index === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    },
  });
}
