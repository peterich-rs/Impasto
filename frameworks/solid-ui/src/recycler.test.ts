import assert from "node:assert/strict";

import {
  discardRecycler,
  syncRecycler,
  type RecyclerSlot,
  type RecyclerState,
} from "./recycler.ts";

type Row = { n: number };

function harness() {
  let next = 1;
  const created: string[] = [];
  const discarded: number[] = [];
  const parents = new Map<number, number>();
  const nextSibling = new Map<number, number>();
  const shown = new Map<number, number>();
  const state: RecyclerState<Row> = { live: new Map(), pools: new Map() };

  function place(node: number, parent: number, anchor: number) {
    if (parents.get(node) === parent && (nextSibling.get(node) ?? 0) === anchor) {
      return;
    }
    parents.set(node, parent);
    nextSibling.set(node, anchor);
  }

  function make(type: string, item: Row, index: number, key: string): RecyclerSlot<Row> {
    const node = next++;
    created.push(type);
    let current = item;
    let at = index;
    const slot: RecyclerSlot<Row> = {
      key,
      type,
      node,
      set(nextItem, nextIndex) {
        current = nextItem;
        at = nextIndex;
        shown.set(node, nextItem.n);
      },
      detach() {
        parents.set(node, 0);
        nextSibling.set(node, 0);
      },
      discard() {
        discarded.push(node);
        parents.delete(node);
      },
      place(parent, anchor) {
        place(node, parent, anchor);
      },
    };
    void current;
    void at;
    shown.set(node, item.n);
    return slot;
  }

  function visual(): number[] {
    const kids = [...parents.entries()].filter(([, parent]) => parent === 1).map(([node]) => node);
    const incoming = new Set<number>();
    for (const node of kids) {
      const sibling = nextSibling.get(node) ?? 0;
      if (sibling !== 0) {
        incoming.add(sibling);
      }
    }
    let head = kids.find((node) => !incoming.has(node));
    const order: number[] = [];
    const guard = kids.length + 1;
    while (head !== undefined && order.length < guard) {
      order.push(shown.get(head) ?? -1);
      const sibling = nextSibling.get(head) ?? 0;
      head = sibling === 0 ? undefined : sibling;
    }
    return order;
  }

  return {
    state,
    created,
    discarded,
    parents,
    visual,
    sync(items: Row[], poolSize = 1) {
      syncRecycler({
        state,
        parent: 1,
        poolSize,
        entries: items.map((item, index) => ({
          key: String(item.n),
          type: item.n % 2 === 0 ? "even" : "odd",
          item,
          index,
        })),
        create(type, item, index) {
          return make(type, item, index, "");
        },
      });
    },
  };
}

const first = harness();
const rows = Array.from({ length: 3 }, (_, n) => ({ n }));
first.sync(rows, 1);
assert.equal(first.created.length, 3);
const createdAfterFirst = first.created.length;
first.sync(rows, 1);
assert.equal(first.created.length, createdAfterFirst);
assert.equal(first.discarded.length, 0);

const slide = harness();
slide.sync([0, 1, 2].map((n) => ({ n })), 1);
slide.sync([1, 2, 3].map((n) => ({ n })), 1);
assert.equal(slide.created.length, 4);
assert.equal(slide.state.live.has("0"), false);
assert.equal(slide.state.live.has("3"), true);
assert.equal(slide.discarded.length, 0);

const again = harness();
again.sync([0, 1].map((n) => ({ n })), 2);
again.sync([], 2);
const beforeReturn = again.created.length;
again.sync([0, 1].map((n) => ({ n })), 2);
assert.equal(again.created.length, beforeReturn);
assert.equal(again.discarded.length, 0);

const capped = harness();
capped.sync([0].map((n) => ({ n })), 0);
capped.sync([2].map((n) => ({ n })), 0);
assert.equal(capped.discarded.length, 1);

const mixed = harness();
mixed.sync([{ n: 1 }], 2);
mixed.sync([{ n: 1 }], 2);
assert.equal(mixed.created.length, 1);
const odd = mixed.state.live.get("1");
assert.ok(odd);
mixed.sync([{ n: 2 }], 2);
assert.equal(mixed.state.live.get("2")?.type, "even");
assert.equal(mixed.state.pools.get("odd")?.length, 1);

discardRecycler(mixed.state);
assert.equal(mixed.state.live.size, 0);
assert.ok(mixed.discarded.length >= 2);

const order = harness();
order.sync([0, 1, 2, 3].map((n) => ({ n })), 8);
assert.deepEqual(order.visual(), [0, 1, 2, 3]);
order.sync([2, 3, 4, 5].map((n) => ({ n })), 8);
assert.deepEqual(order.visual(), [2, 3, 4, 5]);
order.sync([5, 6, 7].map((n) => ({ n })), 8);
assert.deepEqual(order.visual(), [5, 6, 7]);
order.sync([0, 1].map((n) => ({ n })), 8);
assert.deepEqual(order.visual(), [0, 1]);

console.log("recycler.test.ts ok");
