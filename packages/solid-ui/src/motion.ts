import { onFrame, requestFrame, scene } from "@impasto/solid-renderer";
import {
  animateOnClock,
  bindMotion,
  installMotionTestHooks,
  transition,
} from "./motion-clock.ts";
const hooks = {
  reduceMotion: () => {
    return typeof scene.reduceMotion === "function"
      ? scene.reduceMotion()
      : false;
  },
  onFrame,
  requestFrame,
  animate: (
    target: number,
    durationMs: number,
    write: (value: number) => void,
  ) => animateOnClock(hooks, target, durationMs, write),
};
bindMotion(hooks);
export { installMotionTestHooks, transition };
