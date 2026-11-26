import { createSignal, createTrackedEffect, onCleanup, untrack } from "solid-js";
import { QuoteEngine } from "../engine/engine.ts";
import { settingsSig } from "./settings.ts";
const PUBLICATION_MS = 200;

/**
 * App-scope engine. One instance per boot; the gallery app is a
 * single-window process so module scope replaces context here. Pages call
 * `useQuotes()` to keep the market stepping; when the count drops to zero the
 * pipeline stops scheduling market ticks. Timer deadlines wake the UI queue
 * for new data and leave scroll animation free to run between publications.
 */
let engine: QuoteEngine | null = null;
let refs = 0;
const [running, setRunning] = createSignal(true);
const [pulse, setPulse] = createSignal(0);
let timer: ReturnType<typeof setTimeout> | undefined;
let lastTick: number | undefined;
let publicationAt: number | undefined;
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
  if (refs === 0 || !untrack(running) || engine === null) return;
  const now = performance.now();
  const elapsed = lastTick === undefined ? PUBLICATION_MS : now - lastTick;
  lastTick = now;
  engine.setTimeScale(untrack(() => settingsSig().timeScale));
  // Skip catch-up bursts when the app wakes late.
  engine.step(Math.min(PUBLICATION_MS + 100, elapsed));
  setPulse((value) => value + 1);
  publicationAt = (publicationAt ?? now) + PUBLICATION_MS;
  if (publicationAt <= now) publicationAt = now + PUBLICATION_MS;
  syncMarketTimer(publicationAt - now);
}

function syncMarketTimer(delay = 0): void {
  if (refs > 0 && untrack(running)) {
    timer ??= setTimeout(tickMarket, delay);
  } else {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    lastTick = undefined;
    publicationAt = undefined;
  }
}
export type QuotesApi = {
  engine: QuoteEngine;
};

/** Creates the market. Called once by the app shell before pages mount. */
export function wireQuotes(): QuotesApi {
  if (engine === null) {
    engine = new QuoteEngine({
      timeScale: untrack(() => settingsSig().timeScale),
      warmupMinutes: 50,
      dayMs: 6.5 * 3_600_000,
    });
  }
  return {
    engine: engine!,
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

/** Engine access for pages that read quotes on demand rather than per tick. */
export function useBinder(): QuotesApi {
  return wireQuotes();
}
