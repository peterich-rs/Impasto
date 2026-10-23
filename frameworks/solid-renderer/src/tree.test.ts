import { BUTTON_FILL, FONT_SIZE, TEXT_COLOR } from "./defaults.ts";
import type { CoreHost } from "./host.ts";
import { colorBytes } from "./colors.ts";
import { hostRendererOptions } from "./tree.ts";

function fail(message: string): never {
  console.error(message);
  process.exit(1);
}

type Call = { name: string; args: unknown[] };

function mockHost() {
  const calls: Call[] = [];
  let next = 1;
  const texts = new Set<number>();
  let parentOf = 0;
  let first = 0;
  let sibling = 0;
  const note = (name: string, args: unknown[]) => {
    calls.push({ name, args });
  };
  const alloc = (name: string, args: unknown[]) => {
    const id = next++;
    note(name, args);
    if (name === "text") {
      texts.add(id);
    }
    return id;
  };
  const host: CoreHost = {
    column: (gap) => alloc("column", [gap]),
    row: (gap) => alloc("row", [gap]),
    stack: () => alloc("stack", []),
    padding: (edges) => alloc("padding", [edges]),
    grid: (columns, rows, gap) => alloc("grid", [columns, rows, gap]),
    pointerRegion: () => alloc("pointerRegion", []),
    clip: (radius) => alloc("clip", [radius]),
    opacity: (alpha) => alloc("opacity", [alpha]),
    scroll: () => alloc("scroll", []),
    transform: () => alloc("transform", []),
    decorated: (color, radius) => alloc("decorated", [color, radius]),
    image: (source, width, height) => alloc("image", [source, width, height]),
    text: (utf8, fontSize, color) => alloc("text", [utf8, fontSize, color]),
    append: (parent, child) => note("append", [parent, child]),
    insert: (parent, child, before) => note("insert", [parent, child, before]),
    remove: (parent, child) => note("remove", [parent, child]),
    parent: () => parentOf,
    firstChild: () => first,
    nextSibling: () => sibling,
    isText: (node) => texts.has(node),
    setText: (node, utf8) => note("setText", [node, utf8]),
    setFontSize: (node, fontSize) => note("setFontSize", [node, fontSize]),
    setColor: (node, color) => note("setColor", [node, color]),
    setGap: (node, gap) => note("setGap", [node, gap]),
    setPadding: (node, edges) => note("setPadding", [node, edges]),
    setPaddingEdges: (node, top, right, bottom, left) =>
      note("setPaddingEdges", [node, top, right, bottom, left]),
    setMargin: (node, top, right, bottom, left) => note("setMargin", [node, top, right, bottom, left]),
    setMinSize: (node, width, height) => note("setMinSize", [node, width, height]),
    setMaxSize: (node, width, height) => note("setMaxSize", [node, width, height]),
    setShrink: (node, shrink) => note("setShrink", [node, shrink]),
    setBasis: (node, basis) => note("setBasis", [node, basis]),
    setAlignSelf: (node, align) => note("setAlignSelf", [node, align]),
    setGrow: (node, grow) => note("setGrow", [node, grow]),
    setRadius: (node, radius) => note("setRadius", [node, radius]),
    setAlpha: (node, alpha) => note("setAlpha", [node, alpha]),
    setAlign: (node, align) => note("setAlign", [node, align]),
    setSize: (node, width, height) => note("setSize", [node, width, height]),
    setImage: (node, source, width, height) => note("setImage", [node, source, width, height]),
    setMaxLines: (node, lines) => note("setMaxLines", [node, lines]),
    setEllipsis: (node, text) => note("setEllipsis", [node, text]),
    setTextAlign: (node, align) => note("setTextAlign", [node, align]),
    setFontWeight: (node, weight) => note("setFontWeight", [node, weight]),
    setTextDirection: (node, direction) => note("setTextDirection", [node, direction]),
    setLineHeight: (node, height) => note("setLineHeight", [node, height]),
    setFamily: (node, family) => note("setFamily", [node, family]),
    setScrollOffset: (node, x, y) => note("setScrollOffset", [node, x, y]),
    scrollOffset: () => ({ x: 0, y: 0 }),
    rect: () => ({ x: 0, y: 0, w: 0, h: 0 }),
    setTransform: (node, x, y, rotate, scale) => note("setTransform", [node, x, y, rotate, scale]),
    setJustify: (node, justify) => note("setJustify", [node, justify]),
    setWrap: (node, wrap) => note("setWrap", [node, wrap]),
    setFill: (node) => note("setFill", [node]),
    sized: (width, height) => alloc("sized", [width, height]),
    setFocusable: (node, enabled) => note("setFocusable", [node, enabled]),
    focus: (node) => note("focus", [node]),
    blur: (node) => note("blur", [node]),
    setImeEnabled: (enabled) => note("setImeEnabled", [enabled]),
    setImeText: (text) => note("setImeText", [text]),
    measureText: (text, fontSize, maxWidth, color) => {
      note("measureText", [text, fontSize, maxWidth, color]);
      return { w: text.length, h: fontSize };
    },
    requestFrame: () => note("requestFrame", []),
    onClick: (node, fn) => note("onClick", [node, typeof fn]),
    onPress: (node, fn) => note("onPress", [node, typeof fn]),
    onRelease: (node, fn) => note("onRelease", [node, typeof fn]),
    onDragBegin: (node, fn) => note("onDragBegin", [node, typeof fn]),
    onDragBy: (node, fn) => note("onDragBy", [node, typeof fn]),
    onDragEnd: (node, fn) => note("onDragEnd", [node, typeof fn]),
    onFocus: (node, fn) => note("onFocus", [node, typeof fn]),
    onBlur: (node, fn) => note("onBlur", [node, typeof fn]),
    onKey: (node, fn) => note("onKey", [node, typeof fn]),
    onText: (node, fn) => note("onText", [node, typeof fn]),
    onComposing: (node, fn) => note("onComposing", [node, typeof fn]),
    onHover: (node, fn) => note("onHover", [node, typeof fn]),
    onLeave: (node, fn) => note("onLeave", [node, typeof fn]),
    onWheel: (node, fn) => note("onWheel", [node, typeof fn]),
    onSecondaryClick: (node, fn) => note("onSecondaryClick", [node, typeof fn]),
    onFrame: (fn) => note("onFrame", [typeof fn]),
  };
  return {
    host,
    calls,
    setParent(id: number) {
      parentOf = id;
    },
    setFirst(id: number) {
      first = id;
    },
    setSibling(id: number) {
      sibling = id;
    },
  };
}

function find(calls: Call[], name: string): Call | undefined {
  return calls.find((call) => call.name === name);
}

const mock = mockHost();
const options = hostRendererOptions(mock.host);
const column = options.createElement("column", { gap: 8 });
const row = options.createElement("row", { gap: 4 });
const stack = options.createElement("stack");
const padding = options.createElement("padding", { edges: 8 });
const clicked = () => {};
const pointer = options.createElement("pointer", { onClick: clicked });
const decorated = options.createElement("decorated", { color: BUTTON_FILL });
const rounded = options.createElement("decorated", { color: BUTTON_FILL, radius: 6 });
const clip = options.createElement("clip", { radius: 12 });
const fade = options.createElement("opacity", { alpha: 0 });
const logo = options.createElement("image", { source: "asset:logo", w: 56, h: 56 });
const grown = options.createElement("row", { gap: 1, grow: 3 });
const label = options.createTextNode("Hi");

if (find(mock.calls, "column")?.args[0] !== 8) fail("column gap");
if (find(mock.calls, "row")?.args[0] !== 4) fail("row gap");
if (!find(mock.calls, "stack")) fail("stack");
if (find(mock.calls, "padding")?.args[0] !== 8) fail("padding edges");
if (!find(mock.calls, "pointerRegion")) fail("pointer");
if (JSON.stringify(find(mock.calls, "decorated")?.args[0]) !== JSON.stringify(colorBytes(BUTTON_FILL))) {
  fail("decorated color");
}
const decoratedCalls = mock.calls.filter((call) => call.name === "decorated");
if (decoratedCalls[0]?.args[1] !== 0 || decoratedCalls[1]?.args[1] !== 6) fail("decorated radius");
if (find(mock.calls, "clip")?.args[0] !== 12) fail("clip");
if (find(mock.calls, "opacity")?.args[0] !== 0) fail("opacity zero");
const imageCall = find(mock.calls, "image");
if (imageCall?.args[0] !== "asset:logo" || imageCall.args[1] !== 56 || imageCall.args[2] !== 56) {
  fail("image");
}
if (logo.id === 0) fail("image id");
if (find(mock.calls, "setGrow")?.args[0] !== grown.id || find(mock.calls, "setGrow")?.args[1] !== 3) {
  fail("grow");
}
const textCall = find(mock.calls, "text");
if (textCall?.args[0] !== "Hi" || textCall.args[1] !== FONT_SIZE) fail("text node");
if (JSON.stringify(textCall?.args[2]) !== JSON.stringify(colorBytes(TEXT_COLOR))) fail("text color");
if (!mock.host.isText(label.id) || options.isTextNode(column)) fail("isText");

options.insertNode(column, row);
options.insertNode(column, padding, row);
const inserted = mock.calls.filter((call) => call.name === "insert");
if (inserted[0]?.args[0] !== column.id || inserted[0]?.args[1] !== row.id || inserted[0]?.args[2] !== 0) {
  fail("insert without anchor");
}
if (inserted[1]?.args[2] !== row.id) fail("insert with anchor");
options.removeNode(column, padding);
if (find(mock.calls, "remove")?.args[0] !== column.id || find(mock.calls, "remove")?.args[1] !== padding.id) {
  fail("remove");
}

options.setProperty(column, "gap", 12);
options.setProperty(padding, "edges", 3);
options.setProperty(label, "fontSize", 20);
options.setProperty(label, "color", "#112233");
options.setProperty(label, "value", "Yo");
options.setProperty(pointer, "onClick", clicked);
options.setProperty(pointer, "onPress", clicked);
options.setProperty(pointer, "onRelease", clicked);
options.replaceText(label, "Next");

if (find(mock.calls, "setGap")?.args[1] !== 12) fail("setGap");
if (find(mock.calls, "setPadding")?.args[1] !== 3) fail("setPadding");
if (find(mock.calls, "setFontSize")?.args[1] !== 20) fail("setFontSize");
if (JSON.stringify(find(mock.calls, "setColor")?.args[1]) !== JSON.stringify([0x11, 0x22, 0x33, 255])) {
  fail("setColor");
}
const setTexts = mock.calls.filter((call) => call.name === "setText").map((call) => call.args[1]);
if (setTexts[0] !== "Yo" || setTexts[1] !== "Next") fail("setText");
if (find(mock.calls, "onClick")?.args[0] !== pointer.id) fail("onClick");
if (find(mock.calls, "onPress")?.args[0] !== pointer.id) fail("onPress");
if (find(mock.calls, "onRelease")?.args[0] !== pointer.id) fail("onRelease");
options.setProperty(fade, "alpha", 40);
options.setProperty(rounded, "radius", 4);
options.setProperty(label, "maxLines", 2);
options.setProperty(label, "textAlign", "center");
if (find(mock.calls, "setAlpha")?.args[0] !== fade.id || find(mock.calls, "setAlpha")?.args[1] !== 40) {
  fail("setAlpha");
}
if (find(mock.calls, "setRadius")?.args[0] !== rounded.id || find(mock.calls, "setRadius")?.args[1] !== 4) {
  fail("setRadius");
}
if (find(mock.calls, "setMaxLines")?.args[1] !== 2) fail("setMaxLines");
if (find(mock.calls, "setTextAlign")?.args[1] !== "center") fail("setTextAlign");
mock.host.setImage(logo.id, "asset:mark", 12, 8);
const imageUpdates = mock.calls.filter((call) => call.name === "setImage");
if (imageUpdates.length !== 1 || imageUpdates[0]?.args[1] !== "asset:mark") fail("setImage");

mock.setParent(0);
mock.setFirst(0);
mock.setSibling(0);
if (options.getParentNode(row) !== undefined) fail("parent 0");
if (options.getFirstChild(column) !== undefined) fail("first 0");
if (options.getNextSibling(row) !== undefined) fail("sibling 0");
mock.setParent(stack.id);
mock.setFirst(decorated.id);
mock.setSibling(pointer.id);
if (options.getParentNode(row) !== stack) fail("parent");
if (options.getFirstChild(column) !== decorated) fail("first");
if (options.getNextSibling(row) !== pointer) fail("sibling");

const dragged = () => {};
const scroller = options.createElement("scroll", { onDragBy: dragged });
const shifted = options.createElement("transform", { x: 4, scale: 2 });
const spread = options.createElement("row", { gap: 0, justify: "space-between", wrap: true, fill: true });
if (!find(mock.calls, "scroll")) fail("scroll");
if (find(mock.calls, "onDragBy")?.args[0] !== scroller.id) fail("onDragBy");
const transforms = mock.calls.filter((call) => call.name === "setTransform" && call.args[0] === shifted.id);
const shiftedTransform = transforms[transforms.length - 1];
if (shiftedTransform?.args[1] !== 4 || shiftedTransform.args[4] !== 2) fail("transform");
if (find(mock.calls, "setJustify")?.args[0] !== spread.id || find(mock.calls, "setJustify")?.args[1] !== "space-between") {
  fail("justify");
}
if (find(mock.calls, "setWrap")?.args[0] !== spread.id || find(mock.calls, "setWrap")?.args[1] !== true) {
  fail("wrap");
}
if (find(mock.calls, "setFill")?.args[0] !== spread.id) fail("fill");
const field = options.createElement("pointer", { focusable: true, onFocus: clicked });
const box = options.createElement("sized", { w: 1, h: 22 });
if (find(mock.calls, "setFocusable")?.args[0] !== field.id || find(mock.calls, "setFocusable")?.args[1] !== true) {
  fail("focusable");
}
if (find(mock.calls, "onFocus")?.args[0] !== field.id) fail("onFocus");
if (find(mock.calls, "sized")?.args[0] !== 1 || find(mock.calls, "sized")?.args[1] !== 22) fail("sized");
if (box.id === 0) fail("sized id");

const tracks = options.createElement("grid", { columns: ["40", "1fr"], rows: ["min"], gap: 0 });
const gridCall = find(mock.calls, "grid");
if (JSON.stringify(gridCall?.args[0]) !== JSON.stringify(["40", "1fr"])) fail("grid columns");
if (JSON.stringify(gridCall?.args[1]) !== JSON.stringify(["min"])) fail("grid rows");
if (gridCall?.args[2] !== 0 || tracks.id === 0) fail("grid");
const inset = options.createElement("padding", { edges: 4, left: 4, top: 6 });
const edgeCall = mock.calls.filter((call) => call.name === "setPaddingEdges" && call.args[0] === inset.id);
const lastEdge = edgeCall[edgeCall.length - 1];
if (lastEdge?.args[1] !== 6 || lastEdge.args[4] !== 4) fail("padding edges");
options.setProperty(row, "margin", 8);
options.setProperty(row, "shrink", 0);
options.setProperty(row, "alignSelf", "center");
const wheeled = () => {};
options.setProperty(scroller, "onWheel", wheeled);
if (find(mock.calls, "setMargin")?.args[1] !== 8) fail("margin");
if (find(mock.calls, "setShrink")?.args[1] !== 0) fail("shrink");
if (find(mock.calls, "setAlignSelf")?.args[1] !== "center") fail("alignSelf");
if (find(mock.calls, "onWheel")?.args[0] !== scroller.id) fail("onWheel");
const secondary = () => {};
options.setProperty(field, "onSecondaryClick", secondary);
if (find(mock.calls, "onSecondaryClick")?.args[0] !== field.id) fail("onSecondaryClick");

let threw = false;
try {
  options.createElement("nope");
} catch {
  threw = true;
}
if (!threw) fail("unknown tag");

await reorderFor();

console.log("solid-renderer host tree ok");

// `solid-js`'s node export is the server build, which does not rerun effects.
// `node --conditions=browser` loads the client build, where `<For>` reorders.
async function reorderFor(): Promise<void> {
  let solid: typeof import("solid-js");
  let universal: typeof import("@solidjs/universal");
  try {
    solid = await import("solid-js");
    universal = await import("@solidjs/universal");
  } catch (error) {
    const code = error !== null && typeof error === "object" && "code" in error ? error.code : "";
    if (code === "ERR_MODULE_NOT_FOUND") {
      return;
    }
    throw error;
  }
  const resolved = import.meta.resolve("solid-js");
  if (resolved.includes("server.js")) {
    return;
  }

  let next = 1;
  const texts = new Map<number, string>();
  const children = new Map<number, number[]>();
  const parentOf = new Map<number, number>();
  const kids = (parent: number) => {
    let list = children.get(parent);
    if (list === undefined) {
      list = [];
      children.set(parent, list);
    }
    return list;
  };
  const unlink = (child: number) => {
    const parent = parentOf.get(child);
    if (parent === undefined) {
      return;
    }
    const list = children.get(parent);
    if (list !== undefined) {
      const index = list.indexOf(child);
      if (index >= 0) {
        list.splice(index, 1);
      }
    }
    parentOf.delete(child);
  };
  const impl = {
    column: () => next++,
    text: (utf8: string) => {
      const id = next++;
      texts.set(id, utf8);
      return id;
    },
    setText: (node: number, utf8: string) => {
      texts.set(node, utf8);
    },
    isText: (node: number) => texts.has(node),
    append: (parent: number, child: number) => impl.insert(parent, child, 0),
    insert: (parent: number, child: number, before: number) => {
      unlink(child);
      const list = kids(parent);
      const at = before === 0 ? list.length : list.indexOf(before);
      list.splice(at < 0 ? list.length : at, 0, child);
      parentOf.set(child, parent);
    },
    remove: (_parent: number, child: number) => {
      unlink(child);
    },
    parent: (node: number) => parentOf.get(node) ?? 0,
    firstChild: (node: number) => kids(node)[0] ?? 0,
    nextSibling: (node: number) => {
      const parent = parentOf.get(node);
      if (parent === undefined) {
        return 0;
      }
      const list = kids(parent);
      const index = list.indexOf(node);
      return list[index + 1] ?? 0;
    },
  };
  const host = new Proxy(impl, {
    get(target, prop) {
      if (typeof prop === "string" && prop in target) {
        return target[prop as keyof typeof target];
      }
      return () => {
        throw new Error(`unexpected host.${String(prop)}`);
      };
    },
  });
  const options = hostRendererOptions(host);
  const renderer = universal.createRenderer(options);
  const root = options.createElement("column");
  const [items, setItems] = solid.createSignal([
    { id: "a", label: "Alpha" },
    { id: "b", label: "Beta" },
  ]);
  renderer.insert(root, () =>
    solid.For({
      get each() {
        return items();
      },
      children: (item) => options.createTextNode(item.label),
    }),
  );
  const labels = () => kids(root.id).map((id) => texts.get(id));
  if (labels().join(",") !== "Alpha,Beta") {
    fail(`For first order ${labels().join(",")}`);
  }
  const current = items();
  setItems([current[1], current[0]]);
  solid.flush();
  if (labels().join(",") !== "Beta,Alpha") {
    fail(`For reorder ${labels().join(",")}`);
  }
}
