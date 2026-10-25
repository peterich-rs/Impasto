import assert from "node:assert/strict";

import { frameListenerCount, onFrame, runFrames } from "./frames.ts";

const baseline = frameListenerCount();
const stops: Array<() => void> = [];
for (let index = 0; index < 1000; index += 1) {
  stops.push(onFrame(() => {}));
}
assert.equal(frameListenerCount(), baseline + 1000);
for (const stop of stops) {
  stop();
}
assert.equal(frameListenerCount(), baseline);

onFrame(() => {
  throw new Error("tick");
});
assert.equal(frameListenerCount(), baseline + 1);
assert.throws(() => runFrames(0, 16), /tick/);
assert.equal(frameListenerCount(), baseline);
