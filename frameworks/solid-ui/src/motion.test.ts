import assert from "node:assert/strict";
import { animateOnClock, installMotionTestHooks, transition } from "./motion-clock.ts";

let frames = 0;
let writes: number[] = [];
let animated: Array<[number, number]> = [];
installMotionTestHooks({
  reduceMotion: () => true,
  onFrame: () => () => {},
  requestFrame: () => {
    frames += 1;
  },
  animate: (target, durationMs) => {
    animated.push([target, durationMs]);
    return () => {};
  },
});
transition((t) => writes.push(t), 200);
assert.deepEqual(writes, [1]);
assert.equal(frames, 0);
assert.deepEqual(animated, []);

writes = [];
frames = 0;
animated = [];
installMotionTestHooks({
  reduceMotion: () => false,
  onFrame: () => () => {},
  requestFrame: () => {
    frames += 1;
  },
  animate: (target, durationMs) => {
    animated.push([target, durationMs]);
    return () => {};
  },
});
transition((t) => writes.push(t), 0);
assert.deepEqual(writes, [1]);
assert.equal(frames, 0);
assert.deepEqual(animated, []);

writes = [];
animated = [];
let listener: ((targetMs: number, intervalMs: number) => void) | null = null;
const hooks = {
  reduceMotion: () => false,
  onFrame: (fn: (targetMs: number, intervalMs: number) => void) => {
    listener = fn;
    return () => {
      listener = null;
    };
  },
  requestFrame: () => {
    frames += 1;
  },
  animate: (target: number, durationMs: number, write: (value: number) => void) => {
    animated.push([target, durationMs]);
    return animateOnClock(hooks, target, durationMs, write);
  },
};
installMotionTestHooks(hooks);
const stop = transition((t) => writes.push(t), 100);
assert.deepEqual(animated, [[1, 100]]);
listener?.(0, 16);
listener?.(50, 16);
listener?.(100, 16);
assert.deepEqual(writes, [0, 0.5, 1]);
assert.equal(listener, null);
stop();
installMotionTestHooks(null);

console.log("motion.test.ts ok");
