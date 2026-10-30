import assert from "node:assert/strict";
import { createFetch } from "./index.ts";

const signals: AbortSignal[] = [];
const client = createFetch(async (_input, init) => {
  if (init?.signal) {
    signals.push(init.signal);
  }
  return new Response("ok");
});

const first = client.run("/a");
const second = client.run("/b");
await first.catch(() => {});
await second;
assert.equal(signals.length, 2);
assert.equal(signals[0]?.aborted, true);
assert.equal(signals[1]?.aborted, false);

console.log("fetch.test.ts ok");
