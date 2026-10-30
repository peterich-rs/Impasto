import assert from "node:assert/strict";
import { children, createComponent, createRenderEffect, createSignal, createTrackedEffect, flush } from "solid-js";
import { createRenderer } from "@solidjs/universal";

import type { CoreHost } from "../src/host.ts";
import { hostRendererOptions } from "../src/tree.ts";
import { bindSlot } from "./slot.ts";
import { bindText, Text } from "./text.ts";

type Node = { parent: number; children: number[] };

const calls: string[] = [];
const nodes = new Map<number, Node>();
const texts = new Map<number, string>();
const textIds = new Set<number>();
let next = 1;

function alloc(name: string): number {
  const id = next++;
  nodes.set(id, { parent: 0, children: [] });
  if (name === "text") {
    textIds.add(id);
  }
  calls.push(name);
  return id;
}

function unlink(child: number): void {
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

const host = {
  column: () => alloc("column"),
  text: (value?: string) => {
    const id = alloc("text");
    if (value !== undefined) {
      texts.set(id, value);
    }
    return id;
  },
  insert(parent: number, child: number, before: number) {
    unlink(child);
    const parentNode = nodes.get(parent);
    const childNode = nodes.get(child);
    if (parentNode === undefined || childNode === undefined) {
      throw new Error("insert missing");
    }
    childNode.parent = parent;
    if (before === 0) {
      parentNode.children.push(child);
    } else {
      const index = parentNode.children.indexOf(before);
      parentNode.children.splice(index, 0, child);
    }
    calls.push("insert");
  },
  detach(parent: number, child: number) {
    const node = nodes.get(child);
    if (node?.parent !== parent) {
      throw new Error("detach");
    }
    unlink(child);
    calls.push("detach");
  },
  discard(id: number) {
    const walk = (current: number) => {
      const node = nodes.get(current);
      if (node === undefined) {
        return;
      }
      for (const child of [...node.children]) {
        walk(child);
      }
      nodes.delete(current);
    };
    walk(id);
    calls.push("discard");
  },
  remove(parent: number, child: number) {
    if (nodes.get(child)?.parent !== parent) {
      throw new Error("remove");
    }
    unlink(child);
    calls.push("remove");
  },
  parent: (id: number) => nodes.get(id)?.parent ?? 0,
  firstChild: (id: number) => nodes.get(id)?.children[0] ?? 0,
  nextSibling(id: number) {
    const parent = nodes.get(id)?.parent ?? 0;
    if (parent === 0) {
      return 0;
    }
    const children = nodes.get(parent)?.children ?? [];
    return children[children.indexOf(id) + 1] ?? 0;
  },
  setText: (id: number, value: string) => {
    texts.set(id, value);
    calls.push("setText");
  },
  isText: (id: number) => textIds.has(id),
} as unknown as CoreHost;

const scene = hostRendererOptions(host);
bindText(scene);
const { insert } = createRenderer(scene);
const createSlot = bindSlot({
  createElement: scene.createElement,
  insert: (parent, accessor) => {
    insert(parent, accessor);
  },
  forget: (id) => {
    scene.forget(id);
  },
  host,
});

const seen = { flag: false, reads: 0 };
const [flag, setFlag] = createSignal(false);
const slot = createSlot(1, 0, (item) => {
  const text = scene.createTextNode("row");
  createTrackedEffect(() => {
    seen.reads += 1;
    scene.replaceText(text, String(item()));
  });
  createTrackedEffect(() => {
    seen.flag = flag();
  });
  return text;
});
flush();
const created = calls.filter((name) => name === "column" || name === "text").length;
assert.equal(created, 2);
setFlag(true);
slot.set(2, 1);
flush();
assert.equal(seen.flag, true);
assert.equal(calls.filter((name) => name === "column" || name === "text").length, created);
assert.ok(seen.reads >= 2);

const list = alloc("column");
slot.attach(list);
const inserts = calls.filter((name) => name === "insert").length;
slot.place(list, 0);
assert.equal(calls.filter((name) => name === "insert").length, inserts);
slot.detach();
assert.equal(host.parent(slot.node), 0);
slot.discard();
assert.ok(calls.includes("discard"));
assert.equal(scene.retainedCounts().nodes, 0);

function textOf(root: number): string {
  const child = host.firstChild(root);
  return texts.get(child) ?? "";
}

const labeled = createSlot({ n: 1 }, 0, (item) =>
  Text({
    get children() {
      return `Item ${item().n}`;
    },
  }),
);
flush();
const labeledText = host.firstChild(labeled.node);
assert.equal(texts.get(labeledText), "Item 1");
labeled.set({ n: 9 }, 4);
flush();
assert.equal(host.firstChild(labeled.node), labeledText);
assert.equal(texts.get(labeledText), "Item 9");
labeled.discard();

const compiled = createSlot({ n: 3 }, 0, (item) => {
  const row = scene.createElement("column");
  insert(
    row,
    createComponent(Text, {
      get children() {
        return `Item ${item().n}`;
      },
    }),
  );
  return row;
});
flush();
const padding = host.firstChild(compiled.node);
const compiledText = host.firstChild(padding);
assert.equal(texts.get(compiledText), "Item 3");
compiled.set({ n: 12 }, 1);
flush();
assert.equal(texts.get(compiledText), "Item 12");

const memoed = createSlot({ n: 4 }, 0, (item) => {
  const node = scene.createTextNode("");
  const resolved = children(() => `Item ${item().n}`);
  createRenderEffect(
    () => {
      const value = resolved();
      return typeof value === "function" ? value() : value;
    },
    (value) => {
      scene.replaceText(node, String(value ?? ""));
    },
  );
  return node;
});
flush();
assert.equal(textOf(memoed.node), "Item 4");
memoed.set({ n: 7 }, 2);
flush();
assert.equal(textOf(memoed.node), "Item 7");
memoed.discard();
compiled.discard();

const nested = createSlot({ n: 1 }, 0, (item) =>
  Text({
    get children() {
      return `Item ${item().n}`;
    },
  }),
);
const [tick, setTick] = createSignal(0);
createTrackedEffect(() => {
  const n = tick();
  if (n === 0) {
    return;
  }
  nested.set({ n }, n);
});
flush();
const nestedText = host.firstChild(nested.node);
assert.equal(texts.get(nestedText), "Item 1");
setTick(8);
flush();
assert.equal(host.firstChild(nested.node), nestedText);
assert.equal(texts.get(nestedText), "Item 8");
nested.discard();

let owned: ReturnType<typeof createSlot<{ n: number }>> | undefined;
const [generation, setGeneration] = createSignal(1);
createTrackedEffect(() => {
  const n = generation();
  if (owned === undefined) {
    owned = createSlot({ n }, 0, (item) =>
      Text({
        get children() {
          return `Item ${item().n}`;
        },
      }),
    );
    return;
  }
  owned.set({ n }, n);
});
flush();
const ownedText = host.firstChild(owned!.node);
assert.equal(texts.get(ownedText), "Item 1");
setGeneration(9);
flush();
assert.equal(host.firstChild(owned!.node), ownedText);
assert.equal(texts.get(ownedText), "Item 9");
owned!.discard();

console.log("slot.test.ts ok");
