import assert from "node:assert/strict";
import { createRoot, flush } from "solid-js";
import { runFrames } from "../../solid-renderer/runtime/frames.ts";
import { installThemePort, resetThemeForTests } from "./theme.ts";
import { autoFocus, bindFocusRing, popFocusModel, tabInScope, type FocusModel } from "./focus.ts";

const sole: FocusModel = { focused: 2, scopes: [[2]] };
assert.equal(tabInScope(sole).focused, 2);

const nested: FocusModel = { focused: 4, scopes: [[1, 2], [4, 5]] };
assert.equal(tabInScope(nested).focused, 5);
assert.equal(tabInScope(tabInScope(nested)).focused, 4);
assert.deepEqual(tabInScope(nested).scopes, nested.scopes);

const popped = popFocusModel({ focused: 5, scopes: [[1], [5]] }, 1);
assert.equal(popped.focused, 1);
assert.deepEqual(popped.scopes, [[1]]);

const focused: number[] = [];
(globalThis as { core: Record<string, unknown> }).core = {
  focus: (id: number) => {
    focused.push(id);
  },
  requestFrame: () => {},
  setStroke: (id: number, width: number) => {
    strokes.push([id, width]);
  },
};
const strokes: Array<[number, number]> = [];

resetThemeForTests();
installThemePort({
  settings: () => ({
    locale: "en",
    textScale: 1,
    brightness: "light",
    reduceMotion: false,
    boldText: false,
    highContrast: false,
  }),
  onSettings: () => 1,
  offSettings: () => {},
});

createRoot((dispose) => {
  autoFocus(() => 9);
  runFrames(0, 16);
  assert.deepEqual(focused, []);
  runFrames(16, 16);
  assert.deepEqual(focused, [9]);
  bindFocusRing(
    () => 3,
    () => true,
  );
  flush();
  dispose();
});

assert.equal(strokes.some((stroke) => stroke[0] === 3 && stroke[1] === 2), true);
resetThemeForTests();
console.log("focus.test.ts ok");
