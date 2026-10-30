import assert from "node:assert/strict";
import { STACK_EXIT_ALPHA, stackMotion, travelDistance } from "./transition.ts";

assert.deepEqual(stackMotion("enter"), [{ prop: "translateX", from: 100, to: 0 }]);
assert.deepEqual(stackMotion("exit"), [
  { prop: "translateX", from: 0, to: -30 },
  { prop: "opacity", from: 255, to: STACK_EXIT_ALPHA },
]);
assert.deepEqual(stackMotion("enter", 360), [{ prop: "translateX", from: 360, to: 0 }]);
assert.deepEqual(stackMotion("cover", 400), [
  { prop: "translateX", from: 0, to: -120 },
  { prop: "opacity", from: 255, to: STACK_EXIT_ALPHA },
]);
assert.equal(travelDistance(0, 412), 412);
assert.equal(travelDistance(390, 412), 390);
assert.equal(travelDistance(0, 0), 360);

console.log("transition.test.ts ok");
