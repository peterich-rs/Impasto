import { createSignal, onCleanup, untrack } from "solid-js";
import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { QuoteEngine } from "../engine/engine.ts";
import { QuoteBinder } from "./binder.ts";
import { flash } from "./flash.ts";
import { settingsSig } from "./settings.ts";

const UP = [235, 62, 62, 255] as [number, number, number, number];
const DOWN = [34, 176, 95, 255] as [number, number, number, number];
const FLAT = [138, 141, 145, 255] as [number, number, number, number];
const BASE = [30, 34, 40, 255] as [number, number, number, number];

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
let frameHooked = false;

export type QuotesApi = {
  engine: QuoteEngine;
  binder: QuoteBinder;
};

/** Creates the market. Called once by the app shell before pages mount. */
export function wireQuotes(): QuotesApi {
  if (engine === null) {
    engine = new QuoteEngine({ timeScale: untrack(() => settingsSig().timeScale) });
    binder = new QuoteBinder(engine, { up: UP, down: DOWN, flat: FLAT, base: BASE }, () => settingsSig().updown);
    let lastTarget = 0;
    if (!frameHooked) {
      frameHooked = true;
      onFrame((targetMs) => {
        flash.stamp(targetMs);
        const dt = lastTarget === 0 ? 16 : Math.min(100, targetMs - lastTarget);
        lastTarget = targetMs;
        if (untrack(refs) > 0) {
          engine!.step(dt);
          binder!.flush();
          requestFrame();
        }
      });
    }
  }
  return { engine, binder };
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
