import { createSignal, createTrackedEffect, onCleanup, untrack } from "solid-js";
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
 * pipeline stops scheduling market ticks. Timer deadlines wake the UI queue
 * for new data and leave scroll animation free to run between publications.
 */
let engine: QuoteEngine | null = null;
let binder: QuoteBinder | null = null;
let refs = 0;
const [running, setRunning] = createSignal(true);
const [pulse, setPulse] = createSignal(0);
let timer: ReturnType<typeof setTimeout> | undefined;
let lastTick: number | undefined;
let lastPublish = -Infinity;
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
  syncMarketTimer();
}

function tickMarket(): void {
  timer = undefined;
  if (refs === 0 || !untrack(running) || engine === null || binder === null) return;
  const now = performance.now();
  const elapsed = lastTick === undefined ? SIMULATION_TICK_MS : now - lastTick;
  lastTick = now;
  flash.stamp(now);
  engine.setTimeScale(untrack(() => settingsSig().timeScale));
  engine.step(Math.min(100, elapsed));
  binder.flush();
  if (now - lastPublish >= PUBLICATION_MS) {
    lastPublish = now;
    setPulse((value) => value + 1);
  }
  syncMarketTimer(Math.round(SIMULATION_TICK_MS));
}

function syncMarketTimer(delay = 0): void {
  if (refs > 0 && untrack(running)) {
    timer ??= setTimeout(tickMarket, delay);
  } else {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    lastTick = undefined;
  }
}
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
  }
  return {
    engine: engine!,
    binder: binder!,
  };
}

/** Marks this page as a live quote consumer; the market runs while any page needs it. */
export function useQuotes(): QuotesApi {
  const api = wireQuotes();
  refs++;
  syncMarketTimer();
  onCleanup(() => {
    refs--;
    syncMarketTimer();
  });
  return api;
}

/** Binder access without keeping the market stepping (static chrome). */
export function useBinder(): QuotesApi {
  return wireQuotes();
}
