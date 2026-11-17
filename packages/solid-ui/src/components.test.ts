import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

function source(name: string): string {
  return readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
}

const button = source("Button.tsx");
const row = source("Row.tsx");
const field = source("field/shell.tsx");
const dialog = source("Dialog.tsx");
assert.match(button, /fontSize: theme\(\)\.typography\.label\.size/);
assert.match(button, /labelProps\?\.style/);
assert.doesNotMatch(button, /writingDirection/);
assert.match(row, /flexDirection: "row"/);
assert.doesNotMatch(row, /layoutDirection/);
assert.match(field, /inputRef/);
assert.match(field, /controlProps/);
assert.match(field, /errorProps/);
assert.match(dialog, /panelRef\.clear\(\)/);
assert.match(dialog, /panelRef\.attach\(content\)/);
assert.match(source("focus.ts"), /createStyleSource\(node, "component"\)/);
const image = readFileSync(
  new URL("../../solid-renderer/runtime/image.ts", import.meta.url),
  "utf8",
);
assert.match(image, /intrinsicSize/);
assert.doesNotMatch(image, /props\.w/);

console.log("components.test.ts ok");
