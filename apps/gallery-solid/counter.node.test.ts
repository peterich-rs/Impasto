type Call = { name: string; args: unknown[] };

const calls: Call[] = [];
const nodes = new Map<number, { parent: number; children: number[] }>();
const texts = new Set<number>();
let next = 1;

function alloc(name: string, args: unknown[]) {
  const id = next++;
  nodes.set(id, { parent: 0, children: [] });
  calls.push({ name, args });
  if (name === "text") {
    texts.add(id);
  }
  return id;
}

function detach(child: number) {
  const node = nodes.get(child);
  if (node === undefined || node.parent === 0) {
    return;
  }
  const parent = nodes.get(node.parent);
  if (parent !== undefined) {
    parent.children = parent.children.filter((id) => id !== child);
  }
  node.parent = 0;
}

const core = {
  column: (gap: number) => alloc("column", [gap]),
  row: (gap: number) => alloc("row", [gap]),
  stack: () => alloc("stack", []),
  padding: (edges: number) => alloc("padding", [edges]),
  pointerRegion: () => alloc("pointerRegion", []),
  clip: (radius: number) => alloc("clip", [radius]),
  opacity: (alpha: number) => alloc("opacity", [alpha]),
  scroll: () => alloc("scroll", []),
  transform: () => alloc("transform", []),
  decorated: (color: number[], radius = 0) => alloc("decorated", [color, radius]),
  image: (source: string, width: number, height: number) => alloc("image", [source, width, height]),
  text: (utf8: string, fontSize: number, color: number[]) => alloc("text", [utf8, fontSize, color]),
  append: (parent: number, child: number) => {
    core.insert(parent, child, 0);
  },
  insert: (parent: number, child: number, before: number) => {
    const parentNode = nodes.get(parent);
    const childNode = nodes.get(child);
    if (parentNode === undefined || childNode === undefined) {
      throw new Error(`insert missing ${parent} ${child}`);
    }
    detach(child);
    childNode.parent = parent;
    if (before === 0) {
      parentNode.children.push(child);
    } else {
      const index = parentNode.children.indexOf(before);
      if (index < 0) {
        throw new Error("anchor is not a child");
      }
      parentNode.children.splice(index, 0, child);
    }
    calls.push({ name: "insert", args: [parent, child, before] });
  },
  remove: (parent: number, child: number) => {
    const node = nodes.get(child);
    if (node?.parent !== parent) {
      throw new Error("not a child");
    }
    detach(child);
    calls.push({ name: "remove", args: [parent, child] });
  },
  parent: (node: number) => nodes.get(node)?.parent ?? 0,
  firstChild: (node: number) => nodes.get(node)?.children[0] ?? 0,
  nextSibling: (node: number) => {
    const parent = nodes.get(node)?.parent ?? 0;
    if (parent === 0) {
      return 0;
    }
    const children = nodes.get(parent)?.children ?? [];
    const index = children.indexOf(node);
    return children[index + 1] ?? 0;
  },
  isText: (node: number) => texts.has(node),
  setText: (node: number, utf8: string) => {
    calls.push({ name: "setText", args: [node, utf8] });
  },
  setFontSize: (node: number, fontSize: number) => {
    calls.push({ name: "setFontSize", args: [node, fontSize] });
  },
  setColor: (node: number, color: number[]) => {
    calls.push({ name: "setColor", args: [node, color] });
  },
  setGap: (node: number, gap: number) => {
    calls.push({ name: "setGap", args: [node, gap] });
  },
  setPadding: (node: number, edges: number) => {
    calls.push({ name: "setPadding", args: [node, edges] });
  },
  setGrow: (node: number, grow: number) => {
    calls.push({ name: "setGrow", args: [node, grow] });
  },
  setRadius: (node: number, radius: number) => {
    calls.push({ name: "setRadius", args: [node, radius] });
  },
  setAlpha: (node: number, alpha: number) => {
    calls.push({ name: "setAlpha", args: [node, alpha] });
  },
  setAlign: (node: number, align: string) => {
    calls.push({ name: "setAlign", args: [node, align] });
  },
  setSize: (node: number, width: number, height: number) => {
    calls.push({ name: "setSize", args: [node, width, height] });
  },
  setImage: (node: number, source: string, width: number, height: number) => {
    calls.push({ name: "setImage", args: [node, source, width, height] });
  },
  setMaxLines: (node: number, lines: number) => {
    calls.push({ name: "setMaxLines", args: [node, lines] });
  },
  setEllipsis: (node: number, text: string) => {
    calls.push({ name: "setEllipsis", args: [node, text] });
  },
  setTextAlign: (node: number, align: string) => {
    calls.push({ name: "setTextAlign", args: [node, align] });
  },
  setFontWeight: (node: number, weight: number) => {
    calls.push({ name: "setFontWeight", args: [node, weight] });
  },
  setTextDirection: (node: number, direction: string) => {
    calls.push({ name: "setTextDirection", args: [node, direction] });
  },
  setLineHeight: (node: number, height: number) => {
    calls.push({ name: "setLineHeight", args: [node, height] });
  },
  setFamily: (node: number, family: string) => {
    calls.push({ name: "setFamily", args: [node, family] });
  },
  setScrollOffset: (node: number, x: number, y: number) => {
    calls.push({ name: "setScrollOffset", args: [node, x, y] });
  },
  scrollOffset: () => ({ x: 0, y: 0 }),
  rect: () => ({ x: 0, y: 40, w: 100, h: 200 }),
  setTransform: (node: number, x: number, y: number, rotate: number, scale: number) => {
    calls.push({ name: "setTransform", args: [node, x, y, rotate, scale] });
  },
  setJustify: (node: number, justify: string) => {
    calls.push({ name: "setJustify", args: [node, justify] });
  },
  setWrap: (node: number, wrap: boolean) => {
    calls.push({ name: "setWrap", args: [node, wrap] });
  },
  setFill: (node: number) => {
    calls.push({ name: "setFill", args: [node] });
  },
  sized: (width: number, height: number) => alloc("sized", [width, height]),
  setFocusable: (node: number, enabled: boolean) => {
    calls.push({ name: "setFocusable", args: [node, enabled] });
  },
  focus: (node: number) => {
    calls.push({ name: "focus", args: [node] });
  },
  blur: (node: number) => {
    calls.push({ name: "blur", args: [node] });
  },
  setImeEnabled: (enabled: boolean) => {
    calls.push({ name: "setImeEnabled", args: [enabled] });
  },
  setImeText: (text: string) => {
    calls.push({ name: "setImeText", args: [text] });
  },
  measureText: (text: string, fontSize: number, maxWidth: number, color: number[]) => {
    calls.push({ name: "measureText", args: [text, fontSize, maxWidth, color] });
    return { w: text.length, h: fontSize };
  },
  requestFrame: () => {
    calls.push({ name: "requestFrame", args: [] });
  },
  onClick: (node: number, fn: () => void) => {
    calls.push({ name: "onClick", args: [node, typeof fn] });
  },
  onPress: (node: number, fn: () => void) => {
    calls.push({ name: "onPress", args: [node, typeof fn] });
  },
  onRelease: (node: number, fn: () => void) => {
    calls.push({ name: "onRelease", args: [node, typeof fn] });
  },
  onDragBegin: (node: number, fn: () => void) => {
    calls.push({ name: "onDragBegin", args: [node, typeof fn] });
  },
  onDragBy: (node: number, fn: (dx: number, dy: number) => void) => {
    calls.push({ name: "onDragBy", args: [node, typeof fn] });
  },
  onDragEnd: (node: number, fn: (vx: number, vy: number) => void) => {
    calls.push({ name: "onDragEnd", args: [node, typeof fn] });
  },
  onFocus: (node: number, fn: () => void) => {
    calls.push({ name: "onFocus", args: [node, typeof fn] });
  },
  onBlur: (node: number, fn: () => void) => {
    calls.push({ name: "onBlur", args: [node, typeof fn] });
  },
  onKey: (node: number, fn: (key: string) => void) => {
    calls.push({ name: "onKey", args: [node, typeof fn] });
  },
  onText: (node: number, fn: (text: string) => void) => {
    calls.push({ name: "onText", args: [node, typeof fn] });
  },
  onComposing: (node: number, fn: (text: string | null) => void) => {
    calls.push({ name: "onComposing", args: [node, typeof fn] });
  },
  onHover: (node: number, fn: () => void) => {
    calls.push({ name: "onHover", args: [node, typeof fn] });
  },
  onLeave: (node: number, fn: () => void) => {
    calls.push({ name: "onLeave", args: [node, typeof fn] });
  },
  onWheel: (node: number, fn: (dx: number, dy: number) => void) => {
    calls.push({ name: "onWheel", args: [node, typeof fn] });
  },
  onFrame: (fn: () => void) => {
    calls.push({ name: "onFrame", args: [typeof fn] });
  },
};

(globalThis as { core: typeof core }).core = core;

await import(new URL("./dist/counter.js", import.meta.url).href);

function fail(message: string): never {
  console.error(message);
  console.error(calls.map((call) => `${call.name} ${JSON.stringify(call.args)}`).join("\n"));
  process.exit(1);
}

const labels = calls.filter((call) => call.name === "setText").map((call) => call.args[1]);
if (!labels.includes("Count: 0") || !labels.includes("-") || !labels.includes("+")) {
  fail(`missing labels in ${labels.join(", ")}`);
}
if (calls.filter((call) => call.name === "onClick").length !== 53) {
  fail("expected counter clicks, one Open handler per row, and Submit");
}
if (!calls.some((call) => call.name === "scroll")) {
  fail("expected a scroll");
}
if (!calls.some((call) => call.name === "onDragBy" && call.args[1] === "function")) {
  fail("expected a drag handler");
}
if (!calls.some((call) => call.name === "onDragEnd" && call.args[1] === "function")) {
  fail("expected a drag end handler");
}
if (!labels.includes("Item 0") || !labels.includes("Item 49") || !labels.includes("Open")) {
  fail("expected list labels");
}
if (!calls.some((call) => call.name === "onFrame" && call.args[0] === "function")) {
  fail("expected onFrame");
}
if (!calls.some((call) => call.name === "requestFrame")) {
  fail("expected requestFrame");
}
if (!calls.some((call) => call.name === "decorated" && call.args[1] === 6)) {
  fail("expected rounded buttons");
}
if (!calls.some((call) => call.name === "image" && call.args[0] === "asset:logo" && call.args[1] === 56 && call.args[2] === 56)) {
  fail("expected logo image");
}
if (calls.filter((call) => call.name === "onPress").length !== 53 || calls.filter((call) => call.name === "onRelease").length !== 53) {
  fail("expected press handlers on the counter, each row, and Submit");
}
if (calls.filter((call) => call.name === "setFocusable" && call.args[1] === true).length !== 2) {
  fail("expected two focusable fields");
}
if (calls.filter((call) => call.name === "onFocus").length !== 2 || calls.filter((call) => call.name === "onText").length !== 2) {
  fail("expected focus and text handlers on both fields");
}
if (!labels.includes("Name") || !labels.includes("Note") || !labels.includes("Submit")) {
  fail("expected the form");
}
if (!calls.some((call) => call.name === "measureText")) {
  fail("expected caret measurement");
}
console.log("gallery-solid counter render ok");
