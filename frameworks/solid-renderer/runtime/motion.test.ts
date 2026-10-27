import assert from "node:assert/strict";
import { flushMotion, spring } from "./motion.ts";

(globalThis as { core: { requestFrame: () => void } }).core = {
  requestFrame: () => {},
};

let written = -1;
let settled = false;
spring({
  id: 3,
  prop: "opacity",
  from: 0,
  to: 255,
  write: (value) => {
    written = value;
  },
  onDone: () => {
    settled = true;
  },
});
assert.equal(written, 255);
assert.equal(settled, false);
flushMotion();
assert.equal(settled, true);

console.log("motion.test.ts ok");
