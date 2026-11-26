/**
 * Devtools state: Solid signals over the `@impasto/devtools` module.
 * Pulls are cursor-based (each channel tracks the last `seq` it saw) and
 * gated on the hub revision. Frame samples are pulled on demand.
 */
import { createSignal, onCleanup } from "solid-js";
import { appendCapped } from "./cursor.ts";
import type { HighlightBox } from "./highlight.tsx";
import * as raw from "@impasto/devtools/native";
import type { DiagError, FrameSample, GpuFrameTiming, LogLine, NetEntry } from "@impasto/devtools/native";
export type {
  DiagError,
  FrameSample,
  LogLevel,
  LogLine,
  LogSource,
  NetEntry,
  PhaseSample,
} from "@impasto/devtools/native";

export const [logs, setLogs] = createSignal<LogLine[]>([]);
export const [errors, setErrors] = createSignal<DiagError[]>([]);
export const [network, setNetwork] = createSignal<NetEntry[]>([]);
export const [frames, setFrames] = createSignal<FrameSample[]>([]);
export const [traceRecording, setTraceRecording] = createSignal(false);
export const [traceResult, setTraceResult] = createSignal<string | null>(null);
export const [unseenErrors, setUnseenErrors] = createSignal(0);
export const [pageOpen, setPageOpen] = createSignal(false);
export const [highlights, setHighlights] = createSignal<HighlightBox[]>([]);
type Channel = "logs" | "errors" | "network";
const cursors: Record<Channel, number> = {
  logs: 0,
  errors: 0,
  network: 0,
};
let lastRevision = -1;
let framesRevision = -1;
let attached = false;
/** Error source names the banner reacts to. */
const BANNER_SOURCES = new Set(["listener", "rejection", "scene"]);
function tailOf<
  T extends {
    seq: number;
  },
>(current: T[], fresh: T[], cap = 400): T[] {
  return appendCapped(current, fresh, cap);
}
function pull(force = false): void {
  const revision = raw.revision();
  if (!force && revision === lastRevision) {
    return;
  }
  lastRevision = revision;
  const errorLines = raw.errors(cursors.errors).entries;
  if (errorLines.length > 0) {
    cursors.errors = errorLines[errorLines.length - 1].seq;
    setErrors((prev) => tailOf(prev, errorLines));
    setUnseenErrors((n) => n + errorLines.length);
  }
  if (!pageOpen()) return;
  const logLines = raw.logs(cursors.logs).entries;
  if (logLines.length > 0) {
    cursors.logs = logLines[logLines.length - 1].seq;
    setLogs((prev) => tailOf(prev, logLines));
  }
  const netLines = raw.network(cursors.network).entries;
  if (netLines.length > 0) {
    cursors.network = netLines[netLines.length - 1].seq;
    setNetwork((prev) => tailOf(prev, netLines));
  }
}

/** Latest listener/rejection error the banner should surface, if any. */
export function bannerError(): DiagError | null {
  const list = errors();
  for (let index = list.length - 1; index >= 0; index -= 1) {
    const entry = list[index];
    if (entry && BANNER_SOURCES.has(entry.source)) {
      return entry;
    }
  }
  return null;
}
export const store = {
  pageOpen,
  open(): void {
    setPageOpen(true);
    pull(true);
  },
  close(): void {
    setPageOpen(false);
    setHighlights([]);
  },
  clear(): void {
    raw.clear();
    setLogs([]);
    setErrors([]);
    setUnseenErrors(0);
  },
  sampleFrames(): void {
    const page = raw.frames();
    if (page.revision !== framesRevision) {
      framesRevision = page.revision;
      setFrames(page.entries.slice(-120));
    }
  },
  gpuTiming(frame: number): GpuFrameTiming | null {
    return raw.gpuTiming(frame);
  },
  async recordTrace(): Promise<void> {
    if (traceRecording()) return;
    setTraceResult(null);
    try {
      const recording = raw.startTrace();
      setTraceRecording(true);
      await new Promise<void>((resolve) => setTimeout(resolve, 5000));
      const path = await raw.finishTrace(recording);
      setTraceResult(path);
    } catch (error) {
      setTraceResult(`导出失败 · ${String(error)}`);
    } finally {
      setTraceRecording(false);
    }
  },
  /** True when the scene registered the devtools module. */
  available(): boolean {
    try {
      return raw.enabled() === true;
    } catch {
      return false;
    }
  },
  attach(): void {
    if (attached) {
      return;
    }
    attached = true;
    pull();
    const timer = setInterval(() => {
      if (!traceRecording()) pull();
    }, 200);
    onCleanup(() => {
      clearInterval(timer);
      attached = false;
    });
  },
};
