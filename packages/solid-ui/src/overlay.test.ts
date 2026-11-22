import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { installMockHost } from "../testing/renderTest.ts";
import { readFirstChild, restoreChildren } from "./overlay.ts";

function source(name: string): string {
  return readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
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

const mock = installMockHost();
const reparent = mock.host.reparentChildren as (
  from: number,
  to: number,
) => void;
mock.host.reparentChildren = () => {
  throw new Error("reparentChildren failed");
};
assert.doesNotThrow(() => restoreChildren(1, 2));
assert.doesNotThrow(() => restoreChildren(0, 2));
mock.host.reparentChildren = reparent;

const first = mock.host.firstChild as (node: number) => number;
mock.host.firstChild = () => {
  throw new Error("firstChild failed");
};
assert.equal(readFirstChild(4), null);
mock.host.firstChild = first;
assert.equal(readFirstChild(mock.host.createElement("view")), 0);

console.log("overlay.test.ts ok");
