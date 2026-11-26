import { assert } from "impasto:test";
import { readText } from "impasto:test/fixtures";

function source(name: string): string {
  return readText(`./${name}`);
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
  readText("../testing/index.ts"),
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
