import { assert, screen, test } from "impasto:test";
import { readText } from "impasto:test/fixtures";
import { scene } from "@impasto/solid-renderer";
import { readFirstChild, restoreChildren } from "./overlay.ts";

function source(name: string): string {
  return readText(`./${name}`);
}

const dialog = source("Dialog.tsx");
const sheet = source("Sheet.tsx");
const toast = source("Toast.tsx");
const button = source("Button.tsx");

assert.equal(dialog.includes("setAlign("), false);
assert.ok(dialog.includes('justifyContent: "center"'));
assert.equal(sheet.includes("setAlign("), false);
assert.ok(sheet.includes('justifyContent: "end"'));
assert.equal(toast.includes("setAlign("), false);
assert.ok(toast.includes('alignItems: "end"'));
assert.ok(button.includes("minTarget(size(), theme().density) * scale()"));

test("cleans up an overlay with an already disposed child", async () => {
  const parent = scene.createElement("view");
  const child = scene.createElement("view");
  scene.insertNode(parent, child);
  await screen.mount(() => parent);
  assert.equal(readFirstChild(parent), child);
  scene.remove(parent, child);
  assert.doesNotThrow(() => restoreChildren(child, parent));
  assert.doesNotThrow(() => restoreChildren(0, parent));
  assert.equal(readFirstChild(child), 0);
  assert.equal(readFirstChild(parent), 0);
});
