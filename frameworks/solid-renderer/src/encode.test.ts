import { readFileSync } from "node:fs";
import { counterScene } from "./scene.ts";
import { sceneRendererOptions } from "./renderer.ts";

function fail(message: string): never {
  console.error(message);
  process.exit(1);
}

const bytesPath = process.env.UIFW_COUNTER_BYTES;
if (bytesPath === undefined || bytesPath.length === 0) {
  fail("UIFW_COUNTER_BYTES is not set");
}

const expected = readFileSync(bytesPath);
const actual = Buffer.from(counterScene().encode());
if (!actual.equals(expected)) {
  let index = 0;
  const limit = Math.min(actual.length, expected.length);
  while (index < limit && actual[index] === expected[index]) {
    index++;
  }
  fail(`counter bytes differ at ${index}: c=${expected.length} ts=${actual.length}`);
}
console.log(`solid-renderer counter bytes match (${actual.length})`);

const scene = counterScene();
const options = sceneRendererOptions(scene);
const column = options.createElement("layer");
const box = options.createElement("rect", { x: 1, y: 2, w: 3, h: 4, color: "#112233" });
options.insertNode(column, box);
const label = options.createTextNode("Hi");
options.insertNode(column, label);
options.setProperty(label, "color", { r: 1, g: 2, b: 3, a: 4 });

if (options.getFirstChild(column) !== box || options.getNextSibling(box) !== label) {
  fail("child order");
}
if (options.getParentNode(label) !== column || label.parentId !== column.id) {
  fail("parent link");
}
if (!options.isTextNode(label) || options.isTextNode(box)) {
  fail("text node");
}
if (box.r !== 0x11 || box.g !== 0x22 || box.b !== 0x33 || box.a !== 255) {
  fail("color prop");
}
options.removeNode(column, box);
if (options.getFirstChild(column) !== label || box.parentId !== 0) {
  fail("remove");
}
console.log("solid-renderer tree operations ok");
