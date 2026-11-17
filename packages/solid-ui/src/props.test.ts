import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

function source(name: string): string {
  return readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
}

const index = source("index.tsx");
const props = source("props.ts");
const names = [
  "View",
  "Row",
  "Column",
  "Stack",
  "Grid",
  "Text",
  "Image",
  "Icon",
  "Pressable",
  "Button",
  "Switch",
  "Field",
  "TextArea",
  "ScrollArea",
  "VirtualList",
  "Recycler",
  "RefreshControl",
  "NavigationBar",
  "Tabs",
  "Dialog",
  "Sheet",
  "Menu",
  "Popover",
  "Tooltip",
  "Toast",
];
for (const name of names) {
  assert.match(index, new RegExp(`\\b${name}\\b`));
  assert.match(props, new RegExp(`\\b${name}Props\\b`));
}
assert.doesNotMatch(index, /resetThemeForTests/);
assert.doesNotMatch(index, /setDensityForTests/);
assert.match(
  readFileSync(new URL("../testing/index.ts", import.meta.url), "utf8"),
  /resetThemeForTests/,
);
assert.doesNotMatch(source("Button.tsx"), /\bwide\b/);
assert.doesNotMatch(source("Row.tsx"), /\bgap\?:/);
assert.match(source("Button.tsx"), /labelProps/);
assert.match(source("ScrollArea.tsx"), /contentProps/);
assert.match(source("VirtualList.tsx"), /list-content/);
assert.match(source("Pressable.tsx"), /bindPressable/);
assert.match(source("Button.tsx"), /bindPressable/);

console.log("props.test.ts ok");
