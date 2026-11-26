import { assert, screen, test } from "impasto:test";
import { scene } from "@impasto/solid-renderer";
import { autoFocus, bindFocusRing, popFocusModel, tabInScope, type FocusModel } from "./focus.ts";
import { installThemePort } from "./theme.ts";

const sole: FocusModel = { focused: 2, scopes: [[2]] };
assert.equal(tabInScope(sole).focused, 2);

const nested: FocusModel = {
  focused: 4,
  scopes: [
    [1, 2],
    [4, 5],
  ],
};
assert.equal(tabInScope(nested).focused, 5);
assert.equal(tabInScope(tabInScope(nested)).focused, 4);
assert.deepEqual(tabInScope(nested).scopes, nested.scopes);

const popped = popFocusModel({ focused: 5, scopes: [[1], [5]] }, 1);
assert.equal(popped.focused, 1);
assert.deepEqual(popped.scopes, [[1]]);

test("auto focus waits for the inserted node and installs a focus ring", async () => {
  installThemePort(scene);
  let node = 0;
  await screen.mount(() => {
    node = scene.createElement("view", { style: { height: 24 } });
    scene.setProp(node, "focusable", true);
    bindFocusRing(() => node, () => scene.focused() === node);
    autoFocus(() => node);
    return node;
  });
  await screen.advanceFrames(3);
  assert.equal(scene.focused(), node);
});
