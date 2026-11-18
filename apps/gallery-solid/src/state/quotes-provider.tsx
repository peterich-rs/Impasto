import { createSignal, createTrackedEffect, onCleanup, untrack } from "solid-js";
import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { QuoteEngine } from "../engine/engine.ts";
import { QuoteBinder } from "./binder.ts";
import { flash } from "./flash.ts";
import { settingsSig } from "./settings.ts";
const UP = [235, 62, 62, 255] as [number, number, number, number];
const DOWN = [34, 176, 95, 255] as [number, number, number, number];
const FLAT = [138, 141, 145, 255] as [number, number, number, number];
const BASE = [30, 34, 40, 255] as [number, number, number, number];
const SIMULATION_TICK_MS = 1000 / 30;
const PUBLICATION_MS = 200;

/**
 * App-scope engine and binder. One instance per boot; the gallery app is a
 * single-window process so module scope replaces context here. Pages call
 * `useQuotes()` to keep the market stepping; when the count drops to zero the
 * pipeline goes fully silent (no frame requests, no ticks). Binder flush runs
 * after `engine.step` because `runFrames` walks its map in insertion order
 * and the boot frame hook registers first.
 */
let engine: QuoteEngine | null = null;
let binder: QuoteBinder | null = null;
const [refs, setRefs] = createSignal(0);
const [running, setRunning] = createSignal(true);
const [pulse, setPulse] = createSignal(0);
export const marketRunning = running;
export const marketPulse = pulse;

/** Resumes at the next market publication so ending a gesture does not repaint every quote. */
export function useMarketPulse(active: () => boolean): () => number {
  const [revision, setRevision] = createSignal(untrack(marketPulse));
  createTrackedEffect(() => {
    const next = marketPulse();
    if (untrack(active)) setRevision(next);
  });
  return revision;
}

export function toggleMarket(): void {
  setPulse((value) => value + 1);
  setRunning((value) => !value);
  requestFrame();
}
let frameHooked = false;
export type QuotesApi = {
  engine: QuoteEngine;
  binder: QuoteBinder;
};

/** Creates the market. Called once by the app shell before pages mount. */
export function wireQuotes(): QuotesApi {
  if (engine === null) {
    engine = new QuoteEngine({
      timeScale: untrack(() => settingsSig().timeScale),
      warmupMinutes: 50,
      dayMs: 6.5 * 3_600_000,
    });
    binder = new QuoteBinder(
      engine,
      {
        up: UP,
        down: DOWN,
        flat: FLAT,
        base: BASE,
      },
      () => settingsSig().updown,
    );
    let lastTarget: number | undefined;
    let lastPublish = -Infinity;
    if (!frameHooked) {
      frameHooked = true;
      onFrame((targetMs) => {
        flash.stamp(targetMs);
        if (untrack(refs) > 0 && untrack(running)) {
          const elapsed = lastTarget === undefined ? SIMULATION_TICK_MS : targetMs - lastTarget;
          const step = elapsed >= SIMULATION_TICK_MS - 0.25;
          if (step) {
            lastTarget = targetMs;
            engine!.setTimeScale(untrack(() => settingsSig().timeScale));
            engine!.step(Math.min(100, elapsed));
            binder!.flush();
          }
          const age = targetMs - lastPublish;
          // Leave a frame for publication between simulation ticks. A slow
          // display can step on every frame, so bound the deferral.
          if (age >= PUBLICATION_MS && (!step || age >= PUBLICATION_MS + SIMULATION_TICK_MS)) {
            lastPublish = targetMs;
            setPulse((value) => value + 1);
          }
          requestFrame();
        } else {
          lastTarget = undefined;
        }
      });
    }
  }
  return {
    engine: engine!,
    binder: binder!,
  };
}

/** Marks this page as a live quote consumer; the market runs while any page needs it. */
export function useQuotes(): QuotesApi {
  const api = wireQuotes();
  setRefs((n) => n + 1);
  onCleanup(() => setRefs((n) => n - 1));
  return api;
}

/** Binder access without keeping the market stepping (static chrome). */
export function useBinder(): QuotesApi {
  return wireQuotes();
}
