import { onFrame, requestFrame } from "@impasto/solid-renderer";
import { animateOnClock, bindMotion, installMotionTestHooks, transition } from "./motion-clock.ts";

const hooks = {
  reduceMotion: () => {
    const core = (globalThis as { core?: { reduceMotion?: () => boolean } }).core;
    if (typeof core?.reduceMotion !== "function") {
      return false;
    }
    return core.reduceMotion();
  },
  onFrame,
  requestFrame,
  animate: (target: number, durationMs: number, write: (value: number) => void) =>
    animateOnClock(hooks, target, durationMs, write),
};

bindMotion(hooks);

export { installMotionTestHooks, transition };
