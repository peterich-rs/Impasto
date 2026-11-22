import {installStyleMock} from "../testing/style.ts";
import assert from "node:assert/strict";
import type { SceneModule } from "./scene.ts";
import { testScene } from "../testing/scene.ts";

function fail(message: string): never {
  console.error(message);
  process.exit(1);
}

type Call = { name: string; args: unknown[] };

function mockHost() {
  const calls: Call[] = [];
  let next = 4294967296;
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
    if (name === "createTextNode" || (name === "createElement" && args[0] === "text")) {
      texts.add(id);
    }
    return id;
  };
  const host = {
    createElement(tag: string, props?: Record<string, unknown>) {
      return alloc("createElement", [tag, props]);
    },
    createTextNode(value: string | number) {
      return alloc("createTextNode", [String(value)]);
    },
    insertNode: (parent: number, child: number, before = 0) => note("insert", [parent, child, before]),
    setStyle: (node, style) => note("setStyle", [node, style]),
    patchStyle: (node, style) => note("patchStyle", [node, style]),
    clearStyle: (node, keys) => note("clearStyle", [node, keys]),
    setClass: (node, text) => note("setClass", [node, text]),
    setComponentStyle: (node, style) => note("setComponentStyle", [node, style]),
    remove: (parent, child) => note("remove", [parent, child]),
    detach: (parent, child) => note("detach", [parent, child]),
    discard: (node) => note("discard", [node]),
    parent: () => parentOf,
    firstChild: () => first,
    nextSibling: () => sibling,
    isText: (node) => texts.has(node),
    setText: (node, utf8) => note("setText", [node, utf8]),
    setTextRuns: (node, runs) => note("setTextRuns", [node, runs]),
    placePanel: (panel, anchor, align) => note("placePanel", [panel, anchor, align]),
    reparentChildren: (from, to) => note("reparentChildren", [from, to]),
    textCommand: (node, name, extra) => {
      note("textCommand", [node, name, extra]);
      return { text: extra ?? "", start: 0, end: 0 };
    },
    setTextLimit: (node, limit) => note("setTextLimit", [node, limit]),
    setDragClaim: () => {},
    caretAt: () => ({ unit: 0, x: 0, y: 0, w: 1, h: 0 }),
    caretAtUnit: () => ({ x: 0, y: 0, w: 1, h: 0 }),
    selectionRects: () => [],
    wordRange: () => ({ start: 0, end: 0 }),
    focusStep: () => {},
    pushFocusScope: () => {},
    popFocusScope: () => {},
    pointerPosition: () => ({ x: 0, y: 0 }),
    onBack: () => 0,
    offBack: () => {},
    setImage: (node, source, width, height) => note("setImage", [node, source, width, height]),
    setScrollOffset: (node, x, y) => note("setScrollOffset", [node, x, y]),
    overscroll: () => ({ x: 0, y: 0 }),
    setScrollPhysics: (node, tau) => note("setScrollPhysics", [node, tau]),
    setScrollChain: (node, chain) => note("setScrollChain", [node, chain]),
    fling: (node, vx, vy) => note("fling", [node, vx, vy]),
    stopScroll: (node) => note("stopScroll", [node]),
    scrollTo: (node, x, y) => note("scrollTo", [node, x, y]),
    scrollOffset: () => ({ x: 0, y: 0 }),
    rect: () => ({ x: 0, y: 0, w: 0, h: 0 }),
    setFocusable: (node, enabled) => note("setFocusable", [node, enabled]),
    focus: (node) => note("focus", [node]),
    blur: () => note("blur", []),
    focused: () => 0,
    isFocusable: () => false,
    setImeEnabled: (enabled) => note("setImeEnabled", [enabled]),
    setImeText: (text) => note("setImeText", [text]),
    setImeState: (text) => note("setImeState", [text]),
    setImeConfig: (keyboard, action) => note("setImeConfig", [keyboard, action]),
    setPlatformView: (node, kind) => note("setPlatformView", [node, kind]),
    setSemantics: (node, role) => note("setSemantics", [node, role]),
    textScale: () => 1,
    setTextScale: (scale) => note("setTextScale", [scale]),
    reduceMotion: () => false,
    settings: () => ({
      locale: "",
      textScale: 1,
      brightness: "light",
      reduceMotion: false,
      boldText: false,
      highContrast: false,
    }),
    onSettings: () => 0,
    offSettings: () => {},
    graphemeIndex: () => 0,
    graphemeCount: () => 0,
    graphemeUnit: () => 0,

    registerFont: (family, bytes) => note("registerFont", [family, bytes.length]),
    reload: (source) => note("reload", [source]),
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
    onTextReplaced: (node, fn) => note("onTextReplaced", [node, typeof fn]),
    onLink: (node, fn) => note("onLink", [node, typeof fn]),
    onComposing: (node, fn) => note("onComposing", [node, typeof fn]),
    onHover: (node, fn) => note("onHover", [node, typeof fn]),
    onLeave: (node, fn) => note("onLeave", [node, typeof fn]),
    onWheel: (node, fn) => note("onWheel", [node, typeof fn]),
    onScroll: (node, fn) => note("onScroll", [node, typeof fn]),
    onScrollState: (fn) => note("onScrollState", [typeof fn]),
    onSecondaryClick: (node, fn) => note("onSecondaryClick", [node, typeof fn]),
    onLongPress: (node, fn) => note("onLongPress", [node, typeof fn]),
    onZoom: (node, fn) => note("onZoom", [node, typeof fn]),
    setRecognizers: (node, spec) => note("setRecognizers", [node, spec]),
    textEdit: (node, op, text) => {
      note("textEdit", [node, op, text]);
      return { text: text ?? "", start: 0, end: 0 };
    },
    onFrame: (fn) => note("onFrame", [typeof fn]),
    onLayout: (fn) => note("onLayout", [typeof fn]),
  } satisfies Partial<SceneModule>;
  installStyleMock(host,calls);
  (globalThis as unknown as {core:SceneModule}).core=host;
  return {
    host: host as SceneModule,
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
const options = testScene(mock.host);
const clicked = () => {};
const pickedStyle = { flexDirection: "row", gap: 8, flexGrow: 3 };
const row = options.createElement("view", { style: pickedStyle });
const stack = options.createElement("stack", { style: { opacity: 0, overflow: "hidden" } });
const pointer = options.createElement("view", { focusable: true, onClick: clicked });
const imageProps = { source: "asset:logo", intrinsicSize: { width: 56, height: 56 }, required: true };
const logo = options.createElement("image", imageProps);
const label = options.createTextNode("Hi");
const creates = mock.calls.filter((call) => call.name === "createElement");
assert.deepEqual(creates[0]?.args, ["view", { style: pickedStyle }]);
assert.deepEqual(creates[1]?.args, ["stack", { style: { opacity: 0, overflow: "hidden" } }]);
assert.deepEqual(creates[3]?.args, ["image", imageProps]);
assert.deepEqual(find(mock.calls, "createTextNode")?.args, ["Hi"]);
assert.equal(options.isTextNode(label), true);
assert.equal(options.isTextNode(row), false);
assert.deepEqual(find(mock.calls, "setFocusable")?.args, [pointer, true]);
assert.deepEqual(find(mock.calls, "onClick")?.args, [pointer, "function"]);

options.insertNode(stack, row);
options.insertNode(stack, pointer, row);
const inserted = mock.calls.filter((call) => call.name === "insert");
assert.deepEqual(inserted.map((call) => call.args), [[stack, row, 0], [stack, pointer, row]]);
options.removeNode(stack, pointer);
assert.deepEqual(find(mock.calls, "remove")?.args, [stack, pointer]);

const geometry = {
  width: "50%",
  height: "auto",
  paddingLeft: 4,
  paddingTop: 6,
  margin: 8,
  alignSelf: "center",
  position: "absolute",
  top: 10,
  left: 4,
  translateX: 4,
  scaleX: 2,
  gridColumnStart: { span: 2 },
  gridColumnEnd: "auto",
};
options.setProperty(row, "style", geometry);
options.setProperty(label, "style", { fontSize: 20, color: "#112233", maxLines: 2, textAlign: "center" });
options.setProperty(label, "value", "Yo");
options.replaceText(label, "Next");
const styles = mock.calls.filter((call) => call.name === "setStyle");
assert.deepEqual(styles.map((call) => call.args), [
  [row, geometry],
  [label, { fontSize: 20, color: "#112233", maxLines: 2, textAlign: "center" }],
]);
assert.deepEqual(mock.calls.filter((call) => call.name === "setText").map((call) => call.args), [
  [label, "Yo"], [label, "Next"],
]);
mock.host.setImage(logo, "asset:mark", 12, 8);
assert.deepEqual(find(mock.calls, "setImage")?.args, [logo, "asset:mark", 12, 8]);

assert.equal(options.getParentNode(row), undefined);
assert.equal(options.getFirstChild(stack), undefined);
assert.equal(options.getNextSibling(row), undefined);
mock.setParent(stack);
mock.setFirst(logo);
mock.setSibling(pointer);
assert.equal(options.getParentNode(row), stack);
assert.equal(options.getFirstChild(stack), logo);
assert.equal(options.getNextSibling(row), pointer);

const scroller = options.createElement("scroll", { axis: "vertical", onDragBy: clicked });
options.setProperty(scroller, "onWheel", clicked);
options.setProperty(pointer, "onPress", clicked);
options.setProperty(pointer, "onRelease", clicked);
options.setProperty(pointer, "onFocus", clicked);
options.setProperty(pointer, "onSecondaryClick", clicked);
options.setProperty(pointer, "onZoom", clicked);
options.setProperty(pointer, "onLongPress", clicked);
options.setProperty(pointer, "recognizers", { drag: "free", longPress: true });
for (const event of ["onDragBy", "onWheel"]) assert.equal(find(mock.calls, event)?.args[0], scroller);
for (const event of ["onPress", "onRelease", "onFocus", "onSecondaryClick", "onZoom", "onLongPress"]) {
  assert.equal(find(mock.calls, event)?.args[0], pointer);
}
assert.deepEqual(find(mock.calls, "setRecognizers")?.args, [pointer, { drag: "free", longPress: true }]);
assert.throws(() => options.setProperty(row, "w", "50%"), /unknown prop/);

await reorderFor();
await colorStaysPut();
await runsAreSent();

console.log("solid-renderer host tree ok");

// `solid-js`'s node export is the server build, which does not rerun effects.
// `node --conditions=browser` loads the client build, where `<For>` reorders.
async function reorderFor(): Promise<void> {
  let solid: typeof import("solid-js");
  let rendererApi: typeof import("../testing/render.ts");
  try {
    solid = await import("solid-js");
    rendererApi = await import("../testing/render.ts");
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

  let next = 4294967296;
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
    createElement: () => next++,
    createTextNode: (utf8: string) => {
      const id = next++;
      texts.set(id, utf8);
      return id;
    },
    setText: (node: number, utf8: string) => {
      texts.set(node, utf8);
    },
    isText: (node: number) => texts.has(node),
    insertNode: (parent: number, child: number, before = 0) => {
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
  const options = testScene(host);
  const renderer = rendererApi.createSceneRenderer(options);
  const root = options.createElement("view");
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
  const labels = () => kids(root).map((id) => texts.get(id));
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

async function colorStaysPut(): Promise<void> {
  const solid = await import("solid-js");
  const { createSceneRenderer } = await import("../testing/render.ts");
  const { Text, bindText } = await import("../runtime/text.ts");
  const mock = mockHost();
  const options = testScene(mock.host);
  bindText(options);
  const renderer = createSceneRenderer(options);
  const parent = options.createElement("view");
  const [color, setColor] = solid.createSignal("#111111");
  solid.createRoot(() => {
    renderer.render(
      () =>
        Text({
          get style() { return {color:color(),fontSize:16}; },
          children: "Hi",
        }),
      parent,
    );
  });
  solid.flush();
  mock.calls.length = 0;
  setColor("#ffffff");
  solid.flush();
  const updates=mock.calls.filter(call=>call.name==="setStyleSource" && call.args[1]==="inline");
  if (!updates.some(call=>(call.args[2] as {color?:string})?.color==="#ffffff")) fail("missing inline color update");
  if (mock.calls.some(call=>call.name==="setText")) fail("color update rewrote text");
}

async function runsAreSent(): Promise<void> {
  const solid = await import("solid-js");
  const { createSceneRenderer } = await import("../testing/render.ts");
  const { Text, bindText } = await import("../runtime/text.ts");
  const mock = mockHost();
  const options = testScene(mock.host);
  bindText(options);
  const renderer = createSceneRenderer(options);
  const parent = options.createElement("view");
  solid.createRoot(() => {
    renderer.render(
      () =>
        Text({
          style: {fontSize:16},
          runs: [
            {
              text: "Im",
              color: "#112233",
              gradient: {
                kind: "linear",
                from: [0, 0],
                to: [1, 0],
                stops: [
                  { offset: 0, color: "#111111" },
                  { offset: 1, color: [255, 0, 0, 255] },
                ],
              },
            },
          ],
        }),
      parent,
    );
  });
  solid.flush();
  const call = mock.calls.find((entry) => entry.name === "setTextRuns");
  if (call === undefined) {
    fail("missing setTextRuns");
  }
  const runs = call.args[1] as {
    color: string;
    gradient?: { kind: string; stops: { color: string | number[] }[] };
  }[];
  if (runs[0]?.text !== "Im" || runs[0].color !== "#112233") {
    fail(`runs ${JSON.stringify(runs)}`);
  }
  if (runs[0].gradient?.kind !== "linear" || runs[0].gradient.stops[1]?.color[0] !== 255) {
    fail(`gradient ${JSON.stringify(runs[0].gradient)}`);
  }
}
