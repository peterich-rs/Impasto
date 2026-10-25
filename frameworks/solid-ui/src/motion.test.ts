import assert from "node:assert/strict";
import { installMotionTestHooks, transition } from "./motion-clock.ts";

let frames = 0;
let writes: number[] = [];
installMotionTestHooks({
  reduceMotion: () => true,
  onFrame: () => () => {},
  requestFrame: () => {
    frames += 1;
  },
});
transition((t) => writes.push(t), 200);
assert.deepEqual(writes, [1]);
assert.equal(frames, 0);

writes = [];
frames = 0;
installMotionTestHooks({
  reduceMotion: () => false,
  onFrame: () => () => {},
  requestFrame: () => {
    frames += 1;
  },
});
transition((t) => writes.push(t), 0);
assert.deepEqual(writes, [1]);
assert.equal(frames, 0);

writes = [];
let listener: ((targetMs: number, intervalMs: number) => void) | null = null;
installMotionTestHooks({
  reduceMotion: () => false,
  onFrame: (fn) => {
    listener = fn;
    return () => {
      listener = null;
    };
  },
  requestFrame: () => {
    frames += 1;
  },
});
const stop = transition((t) => writes.push(t), 100);
assert.equal(frames, 1);
listener?.(0, 16);
listener?.(50, 16);
listener?.(100, 16);
assert.deepEqual(writes, [0, 0.5, 1]);
assert.equal(listener, null);
stop();
installMotionTestHooks(null);

console.log("motion.test.ts ok");
