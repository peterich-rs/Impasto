/**
 * Devtools state: Solid signals over the `@impasto/devtools/host` module.
 * Pulls are cursor-based (each channel tracks the last `seq` it saw) and
 * gated on the hub revision so a closed panel costs one atomic read.
 */
import { createSignal, onCleanup } from "solid-js";
import { onFrame } from "@impasto/solid-renderer";
import { appendCapped } from "./cursor.ts";
import type { HighlightBox } from "./highlight.tsx";
import * as raw from "./host.gen.ts";

export type LogLevel = 0 | 1 | 2 | 3 | 4;
export type LogSource = "script" | "layout" | "host" | "net";

export type LogLine = {
  seq: number;
  at: number;
  level: LogLevel;
  source: LogSource;
  text: string;
};

export type DiagError = {
  seq: number;
  at: number;
  source: string;
  message: string;
  stack: string | null;
  frame: number | null;
};

export type NetEntry = {
  seq: number;
  at: number;
  kind: "http" | "stream" | "socket";
  token: number;
  method: string;
  url: string;
  requestHeaders: [string, string][];
  requestBody: string | null;
  status: number | null;
  responseHeaders: [string, string][];
  responseBytes: number | null;
  responsePreview: string | null;
  error: string | null;
  doneAt: number | null;
};

export type PhaseSample = {
  motionMs: number;
  dispatchMs: number;
  timerMs: number;
  rafMs: number;
  callbackMs: number;
  flushMs: number;
  layoutCbMs: number;
  syncMs: number;
  semanticsMs: number;
  layworkMs: number;
  passes: number;
  reuse: number;
  shaped: number;
  pointers: number;
  keys: number;
  edits: number;
  ime: number;
  scrolls: number;
  focus: number;
  blur: number;
};

export type FrameSample = {
  seq: number;
  number: number;
  at: number;
  sessionMs: number;
  gestureMs: number;
  scriptMs: number;
  layoutMs: number;
  recordMs: number;
  submitMs: number;
  presentMs: number;
  layers: number;
  paragraphs: number;
  pictures: number;
  textures: number;
  overBudget: boolean;
  phases?: PhaseSample;
};

export const [logs, setLogs] = createSignal<LogLine[]>([]);
export const [errors, setErrors] = createSignal<DiagError[]>([]);
export const [network, setNetwork] = createSignal<NetEntry[]>([]);
export const [frames, setFrames] = createSignal<FrameSample[]>([]);
export const [unseenErrors, setUnseenErrors] = createSignal(0);
export const [pageOpen, setPageOpen] = createSignal(false);
export const [highlights, setHighlights] = createSignal<HighlightBox[]>([]);

type Channel = "logs" | "errors" | "network" | "frames";
const cursors: Record<Channel, number> = { logs: 0, errors: 0, network: 0, frames: 0 };
let lastRevision = -1;
let attached = false;
/** Error source names the banner reacts to. */
const BANNER_SOURCES = new Set(["listener", "rejection", "host"]);

function entriesOf<T extends { seq: number }>(payload: unknown): T[] {
  if (!payload || typeof payload !== "object") {
    return [];
  }
  const entries = (payload as { entries?: unknown }).entries;
  return Array.isArray(entries) ? (entries as T[]) : [];
}

function tailOf<T extends { seq: number }>(current: T[], fresh: T[], cap = 400): T[] {
  return appendCapped(current, fresh, cap);
}

function pull(): void {
  const revision = raw.revision() as number;
  if (revision === lastRevision) {
    return;
  }
  lastRevision = revision;
  const logLines = entriesOf<LogLine>(raw.logs(cursors.logs));
  if (logLines.length > 0) {
    cursors.logs = logLines[logLines.length - 1].seq;
    setLogs((prev) => tailOf(prev, logLines));
  }
  const errorLines = entriesOf<DiagError>(raw.errors(cursors.errors));
  if (errorLines.length > 0) {
    cursors.errors = errorLines[errorLines.length - 1].seq;
    setErrors((prev) => tailOf(prev, errorLines));
    setUnseenErrors((n) => n + errorLines.length);
  }
  const netLines = entriesOf<NetEntry>(raw.network(cursors.network));
  if (netLines.length > 0) {
    cursors.network = netLines[netLines.length - 1].seq;
    setNetwork((prev) => tailOf(prev, netLines));
  }
  const frameLines = entriesOf<FrameSample>(raw.frames(cursors.frames));
  if (frameLines.length > 0) {
    cursors.frames = frameLines[frameLines.length - 1].seq;
    setFrames((prev) => tailOf(prev, frameLines, 240));
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
    pull();
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
  /** True when the host registered the devtools module. */
  available(): boolean {
    try {
      const value = raw.enabled() as boolean | undefined;
      return value === true;
    } catch {
      return false;
    }
  },
  attach(): void {
    if (attached) {
      return;
    }
    attached = true;
    let skip = 0;
    const stop = onFrame(() => {
      skip = (skip + 1) % 4;
      if (skip !== 0 && !pageOpen()) {
        return;
      }
      pull();
    });
    onCleanup(stop);
  },
};
