import assert from "node:assert/strict";
import { installMockHost } from "../../packages/solid-ui/testing/renderTest.ts";
import type { CanvasRenderingContext2D } from "@impasto/solid-renderer";
import type { Bar } from "./src/engine/types.ts";

installMockHost();
const { buildChartGeometry, paintChart, resolveSelection } = await import("./src/widgets/stocks/chart-model.ts");

function bars(count: number): Bar[] {
  return Array.from({ length: count }, (_, index) => ({
    t: index * 60_000,
    o: 100 + index,
    h: 110 + index,
    l: 90 + index,
    c: 105 + index,
    v: 1_000 + index,
  }));
}

const slot = 6;
const daily = buildChartGeometry({
  bars: bars(10),
  period: "1D",
  startOffset: 0,
  slot,
  width: 300,
  height: 180,
  previous: 100,
});
assert.ok(daily);
assert.equal(daily.hit(0), 0);
assert.equal(daily.hit(daily.plot - 0.01), 9);
assert.equal(daily.hit(-1), null);
assert.equal(daily.hit(daily.plot + 1), null);

const sample = bars(80);
const open = buildChartGeometry({ bars: sample, period: "1D", startOffset: 0, slot, width: 132, height: 180, previous: 100 });
assert.ok(open);
assert.equal(open.first, 67);
assert.equal(open.end, 80);
assert.equal(resolveSelection(sample[75]!.t, sample, open.first, open.end), 75);

const panned = buildChartGeometry({ bars: sample, period: "1D", startOffset: 20, slot, width: 132, height: 180, previous: 100 });
assert.ok(panned);
assert.equal(panned.first, 47);
assert.equal(panned.end, 60);
assert.equal(resolveSelection(sample[75]!.t, sample, panned.first, panned.end), null);
assert.equal(resolveSelection(sample[50]!.t, sample, panned.first, panned.end), 50);

const far = buildChartGeometry({ bars: sample, period: "1D", startOffset: 70, slot, width: 132, height: 180, previous: 100 });
assert.ok(far);
assert.equal(far.end, 13);
assert.equal(far.first, 0);

assert.equal(resolveSelection(null, sample, 0, sample.length), null);
const duplicated = bars(4);
duplicated[2] = { ...duplicated[1]! };
assert.equal(resolveSelection(duplicated[1]!.t, duplicated, 0, 4), 1);

assert.equal(buildChartGeometry({ bars: sample, period: "1D", startOffset: 0, slot, width: 90, height: 180, previous: 100 }), null);
assert.equal(buildChartGeometry({ bars: sample, period: "1D", startOffset: 0, slot, width: 300, height: 80, previous: 100 }), null);
assert.equal(buildChartGeometry({ bars: [], period: "1D", startOffset: 0, slot, width: 300, height: 180, previous: 100 }), null);

function recorder(): { ctx: CanvasRenderingContext2D; texts: string[] } {
  const texts: string[] = [];
  const gradient = { addColorStop() {} };
  const ctx = {
    font: "",
    textBaseline: "middle" as const,
    textAlign: "left" as const,
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    lineJoin: "miter" as const,
    beginPath() {},
    polyline() {},
    lineSegments() {},
    fillRects() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    fill() {},
    closePath() {},
    save() {},
    restore() {},
    clip() {},
    rect() {},
    fillRect() {},
    strokeRect() {},
    fillText(text: string) { texts.push(text); },
    setLineDash() {},
    roundRect() {},
    createLinearGradient() { return gradient; },
    arc() {},
  };
  return { ctx: ctx as CanvasRenderingContext2D, texts };
}

const painted = recorder();
paintChart(painted.ctx, daily);
assert.equal(painted.texts.some((text) => text.startsWith("成交量")), false);

const minutes = bars(8).map((bar) => ({ ...bar, avg: bar.c - 0.2, turnover: bar.c * bar.v }));
const intraday = buildChartGeometry({
  bars: minutes,
  period: "intraday",
  startOffset: 0,
  slot,
  width: 300,
  height: 180,
  previous: 100,
});
assert.ok(intraday);
assert.equal(intraday.first, 0);
assert.equal(intraday.end, minutes.length);
assert.equal(intraday.hit(0), 0);
assert.equal(intraday.hit(-1), null);
const intradayPaint = recorder();
paintChart(intradayPaint.ctx, intraday);
assert.equal(intradayPaint.texts.some((text) => text.startsWith("成交量")), false);
