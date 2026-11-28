import { assert, test } from "impasto:test";
import { frameDroppedVsyncs, frameHitchMs, frameStageMs, summarizeFrames, type MetricsFrame } from "./metrics.ts";

const budget = 1000 / 60;
function frame(at: number, changes: Partial<MetricsFrame> = {}): MetricsFrame {
  return {
    seq: at, at, uiMs: 3, rasterMs: 4, gpuSpanMs: 1, e2eMs: 8, workMs: 8,
    budgetMs: budget, presented: true, trigger: "wake",
    uiOverBudget: false, rasterOverBudget: false, gpuOverBudget: false, ...changes,
  };
}

test("periodic repaints report 5 presents per second without hitch or burst", () => {
  const live = { sampledAt: 2000, firstFrameAt: 0 };
  const empty = summarizeFrames([], budget, live);
  assert.equal(empty.idle, true);
  assert.equal(empty.onTimePct, 100);
  const samples = Array.from({ length: 10 }, (_, i) => frame(i * 200));
  const stats = summarizeFrames(samples, budget, live);
  assert.equal(stats.hitchMsPerSec, 0);
  assert.equal(stats.presentRateHz, 5);
  assert.equal(stats.wakeCount, 10);
  assert.equal(stats.burstFps, undefined);
});

test("a recent 60 Hz burst retains its cadence among periodic wakes", () => {
  const samples = [
    ...Array.from({ length: 7 }, (_, i) => frame(i * 200)),
    ...Array.from({ length: 30 }, (_, i) => frame(1500 + i * budget, { trigger: "vsync" })),
  ];
  const live = { sampledAt: 2000, firstFrameAt: 0 };
  assert.equal(summarizeFrames(samples, budget, live).presentRateHz, 18.5);
  assert.ok(Math.abs((summarizeFrames(samples, budget, live).burstFps ?? 0) - 60) < 1e-9);
  samples[15] = { ...samples[15], presented: false, skipReason: "queue_full" };
  const dropped = summarizeFrames(samples, budget, live);
  assert.ok(Math.abs((dropped.burstFps ?? 0) - 58) < 1e-9);
  assert.equal(dropped.droppedFrames, 1);
  const idle = summarizeFrames(samples, budget, { ...live, sampledAt: 2500 });
  assert.equal(idle.idle, true);
  assert.equal(idle.presentRateHz, 0);
  assert.equal(idle.burstFps, undefined);
  assert.equal(idle.hitchMsPerSec, dropped.hitchMsPerSec);
});

test("overlapping split stages have a 10 ms bottleneck and 20 ms latency", () => {
  const samples = Array.from({ length: 60 }, (_, i) =>
    frame(i * budget, { uiMs: 10, rasterMs: 10, gpuSpanMs: 8, e2eMs: 20, workMs: 20 }));
  const stats = summarizeFrames(samples, budget, { sampledAt: 1000, firstFrameAt: 0 });
  assert.equal(stats.p95Ms, 10);
  assert.equal(stats.p95E2eMs, 20);
  assert.equal(stats.presentRateHz, 60);
  assert.equal(stats.hitchMsPerSec, 0);
  assert.equal(stats.droppedFrames, 0);
});

test("shared CPU work, raster and GPU overruns use the same drop and hitch units", () => {
  const shared = frame(0, { topology: "shared", uiMs: 10, rasterMs: 8, budgetMs: 0 });
  const gpu = frame(10, { gpuSpanMs: 49 });
  const raster = frame(20, { rasterMs: 50 });
  assert.equal(frameStageMs(shared), 18);
  assert.equal(frameDroppedVsyncs(shared, budget), 1);
  assert.equal(frameHitchMs(shared, budget), budget);
  assert.equal(frameDroppedVsyncs(gpu), 2);
  assert.equal(frameDroppedVsyncs(raster), 2);
  const stats = summarizeFrames([shared, gpu, raster], budget, { sampledAt: 1000, firstFrameAt: 0 });
  assert.equal(stats.droppedFrames, 5);
  assert.equal(stats.uiJankCount, 1);
  assert.equal(stats.rasterJankCount, 1);
  assert.equal(stats.gpuJankCount, 1);
  assert.ok(Math.abs(stats.hitchMsPerSec - 83.3333) < 0.001);
});
