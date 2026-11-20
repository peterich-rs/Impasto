import { createRoot, createTrackedEffect, untrack } from "solid-js";
import { scene } from "./scene.ts";

export type BackGesture =
  | { phase: "start"; fromRight: boolean }
  | { phase: "progress"; progress: number }
  | { phase: "cancel" }
  | { phase: "commit" };

type Options = { enabled?: () => boolean; gesture?: (event: BackGesture) => void };
type Entry = { priority: number; seq: number; fn: () => boolean; enabled: boolean; gesture?: Options["gesture"] };

let entries: Entry[] = [];
let seq = 0;
let hostId = 0;
let installed = false;
let target: Entry | undefined;
let published: boolean | undefined;

function ordered() {
  return entries.filter((entry) => entry.enabled)
    .sort((left, right) => right.priority - left.priority || right.seq - left.seq);
}

function publish() {
  const enabled = entries.some((entry) => entry.enabled);
  if (enabled !== published) {
    published = enabled;
    scene.setBackEnabled?.(enabled);
  }
}

function dispatchGesture(phase: BackGesture["phase"], value: number) {
  if (phase === "start") {
    target?.gesture?.({ phase: "cancel" });
    target = ordered()[0];
    target?.gesture?.({ phase, fromRight: value !== 0 });
  } else if (phase === "progress") {
    target?.gesture?.({ phase, progress: value });
  } else {
    const entry = target;
    target = undefined;
    if (phase === "commit" && entry?.enabled) {
      entry.gesture?.({ phase });
      if (entry.fn()) return true;
      return ordered().some((candidate) =>
        (candidate.priority < entry.priority ||
          (candidate.priority === entry.priority && candidate.seq < entry.seq)) && candidate.fn()
      );
    } else {
      entry?.gesture?.({ phase: "cancel" });
    }
  }
  return true;
}

/**
 * Register `fn`. Higher `priority` runs first. The same priority runs
 * latest-first. Returns a release function. One `scene.onBack` for the process.
 */
export function onBack(priority: number, fn: () => boolean, options: Options = {}): () => void {
  if (typeof scene.onBack !== "function") {
    return () => {};
  }
  if (!installed) {
    hostId = scene.onBack(() => {
      for (const entry of ordered()) {
        if (entry.fn()) {
          return true;
        }
      }
      return false;
    });
    scene.onBackGesture?.(dispatchGesture);
    installed = true;
  }
  const entry: Entry = {
    priority, seq: ++seq, fn, enabled: options.enabled ? untrack(options.enabled) : true,
    gesture: options.gesture,
  };
  entries.push(entry);
  const enabled = options.enabled;
  const dispose = enabled && createRoot((dispose) => {
    createTrackedEffect(() => {
      entry.enabled = enabled();
      publish();
    });
    return dispose;
  });
  publish();
  return () => {
    dispose?.();
    if (target === entry) {
      target = undefined;
      entry.gesture?.({ phase: "cancel" });
    }
    entries = entries.filter((item) => item !== entry);
    publish();
    if (
      entries.length === 0 &&
      installed &&
      typeof scene.offBack === "function"
    ) {
      scene.offBack(hostId);
      installed = false;
      hostId = 0;
    }
  };
}
