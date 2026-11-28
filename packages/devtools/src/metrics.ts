import type { FrameSample } from "@impasto/devtools/native";

// The same frame and window formulas are used by the Desktop Rust store.
export type MetricsFrame = Pick<FrameSample,
  | "seq" | "at" | "uiMs" | "rasterMs" | "gpuSpanMs" | "e2eMs" | "workMs"
  | "budgetMs" | "topology" | "compositorOnly" | "presented" | "skipReason"
  | "trigger" | "uiOverBudget" | "rasterOverBudget" | "gpuOverBudget"
>;

const LIVE_WINDOW_MS = 2000;
const IDLE_AFTER_MS = 500;

function frameUiMs(frame: MetricsFrame): number {
  return frame.compositorOnly ? 0 : frame.uiMs;
}

function frameCpuMs(frame: MetricsFrame): number {
  const ui = frameUiMs(frame);
  return frame.topology === "shared"
    ? Math.max(ui + frame.rasterMs, frame.workMs)
    : Math.max(ui, frame.rasterMs);
}

function frameGpuMs(frame: MetricsFrame): number {
  return Math.max(0, frame.gpuSpanMs ?? 0);
}

export function frameStageMs(frame: MetricsFrame): number {
  return Math.max(frameCpuMs(frame), frameGpuMs(frame));
}

export function effectiveBudget(frame: MetricsFrame, fallbackBudget: number): number {
  if (Number.isFinite(frame.budgetMs) && frame.budgetMs > 0) return frame.budgetMs;
  return Number.isFinite(fallbackBudget) && fallbackBudget > 0 ? fallbackBudget : 0;
}

export function frameStageFlags(frame: MetricsFrame, fallbackBudget = 0) {
  const budget = effectiveBudget(frame, fallbackBudget);
  const uiMs = frameUiMs(frame);
  const rasterMs = frame.rasterMs;
  const shared = frame.topology === "shared";
  const cpuOver = shared && budget > 0 && frameCpuMs(frame) > budget;
  const uiDominant = !frame.compositorOnly && uiMs >= rasterMs;
  const ui = !frame.compositorOnly && (frame.uiOverBudget ||
    (shared ? cpuOver && uiDominant : budget > 0 && uiMs > budget));
  const raster = frame.rasterOverBudget ||
    (shared ? cpuOver && !uiDominant : budget > 0 && rasterMs > budget);
  const gpu = frame.gpuOverBudget || (budget > 0 && frameGpuMs(frame) > budget);
  return { ui, raster, gpu, any: ui || raster || gpu };
}

function failedOrSkipped(frame: MetricsFrame): boolean {
  return !frame.presented || frame.skipReason != null;
}

export function frameDroppedVsyncs(frame: MetricsFrame, fallbackBudget = 0): number {
  const budget = effectiveBudget(frame, fallbackBudget);
  const stage = frameStageMs(frame);
  const lost = budget > 0 && stage > budget ? Math.max(1, Math.ceil(stage / budget) - 1) : 0;
  return Math.max(lost, Number(failedOrSkipped(frame) || frameStageFlags(frame, fallbackBudget).any));
}

export function frameHitchMs(frame: MetricsFrame, fallbackBudget = 0): number {
  return frameDroppedVsyncs(frame, fallbackBudget) * effectiveBudget(frame, fallbackBudget);
}

function burstFps(entries: readonly MetricsFrame[], budget: number, fallbackBudget: number): number | undefined {
  if (budget <= 0) return undefined;
  const gapMs = Math.max(2.5 * budget, 50);
  let start = 0;
  let previous: number | undefined;
  let presented = 0;
  let burst: { start: number; end: number; presented: number } | undefined;
  for (const [index, frame] of entries.entries()) {
    if (!frame.presented) continue;
    if (previous === undefined || frame.at - previous > gapMs) {
      start = index;
      presented = 0;
    }
    previous = frame.at;
    presented += 1;
    if (presented >= 3) burst = { start, end: index, presented };
  }
  if (!burst) return undefined;
  const dropped = entries.slice(burst.start, burst.end + 1)
    .reduce((count, frame) => count + frameDroppedVsyncs(frame, fallbackBudget), 0);
  return 1000 / budget * burst.presented / (burst.presented + dropped);
}

export function summarizeFrames(
  entries: readonly MetricsFrame[],
  fallbackBudget = 0,
  live?: { sampledAt: number; firstFrameAt?: number | null },
) {
  const sorted = entries.filter(frame => !live || frame.at >= live.sampledAt - LIVE_WINDOW_MS)
    .sort((a, b) => a.at - b.at || a.seq - b.seq);
  const first = sorted[0];
  const last = sorted.at(-1);
  const count = sorted.length;
  const budget = last ? effectiveBudget(last, fallbackBudget) : fallbackBudget;
  const spanMs = live
    ? Math.min(LIVE_WINDOW_MS, Math.max(250, live.sampledAt - (live.firstFrameAt ?? live.sampledAt)))
    : Math.max((last?.at ?? 0) - (first?.at ?? 0), budget, 1);
  let hitchMs = 0;
  let droppedFrames = 0;
  let presentedCount = 0;
  let latestPresentedAt = -Infinity;
  let jankCount = 0;
  let uiJankCount = 0;
  let rasterJankCount = 0;
  let gpuJankCount = 0;
  let wakeCount = 0;
  const work: number[] = [];
  const latencies: number[] = [];
  for (const frame of sorted) {
    const flags = frameStageFlags(frame, fallbackBudget);
    uiJankCount += Number(flags.ui);
    rasterJankCount += Number(flags.raster);
    gpuJankCount += Number(flags.gpu);
    jankCount += Number(flags.any || failedOrSkipped(frame));
    droppedFrames += frameDroppedVsyncs(frame, fallbackBudget);
    hitchMs += frameHitchMs(frame, fallbackBudget);
    wakeCount += Number(frame.trigger === "wake");
    if (frame.presented) {
      presentedCount += 1;
      latestPresentedAt = frame.at;
    }
    work.push(frameStageMs(frame));
    latencies.push(frame.e2eMs);
  }
  const idle = live != null && live.sampledAt - latestPresentedAt >= IDLE_AFTER_MS;
  work.sort((a, b) => a - b);
  latencies.sort((a, b) => a - b);
  const percentile = (samples: number[], fraction: number) => samples[Math.ceil(count * fraction) - 1] ?? 0;
  return {
    count, idle, droppedFrames, presentedCount, jankCount, uiJankCount, rasterJankCount, gpuJankCount, wakeCount,
    hitchMs,
    hitchMsPerSec: hitchMs * 1000 / spanMs,
    onTimePct: count ? (count - jankCount) / count * 100 : 100,
    presentRateHz: idle ? 0 : presentedCount * 1000 / spanMs,
    burstFps: idle ? undefined : burstFps(sorted, budget, fallbackBudget),
    p50Ms: percentile(work, 0.50),
    p95Ms: percentile(work, 0.95),
    p95E2eMs: percentile(latencies, 0.95),
    peakMs: work.at(-1) ?? 0,
  };
}
