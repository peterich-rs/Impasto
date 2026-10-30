import assert from "node:assert/strict";

import { applyListOp, type ListBindingSlot, type ListBindingState } from "./list-binding.ts";

type Row = { n: number };

function harness(poolSize = 1) {
  let next = 1;
  const created: string[] = [];
  const discarded: number[] = [];
  const state: ListBindingState<Row> = { live: new Map(), pools: new Map() };
  const items = new Map<number, Row>();

  function make(type: string, item: Row, index: number, key: string): ListBindingSlot<Row> {
    const node = next++;
    created.push(type);
    const slot: ListBindingSlot<Row> = {
      key,
      type,
      node,
      set(nextItem) {
        items.set(node, nextItem);
      },
      detach() {},
      discard() {
        discarded.push(node);
      },
      place() {},
    };
    items.set(node, item);
    void index;
    return slot;
  }

  return {
    created,
    discarded,
    state,
    op(
      op: Parameters<typeof applyListOp<Row>>[1],
      limit = poolSize,
    ) {
      applyListOp(state, op, {
        parent: () => 1,
        poolSize: () => limit,
        itemAt: (index) => ({ n: index }),
        create: make,
        note() {},
      });
    },
  };
}

const first = harness(1);
first.op({ kind: "adopt", key: "0", type: "even", index: 0 });
first.op({ kind: "adopt", key: "1", type: "odd", index: 1 });
first.op({ kind: "adopt", key: "2", type: "even", index: 2 });
assert.equal(first.created.length, 3);
first.op({ kind: "set", key: "1", index: 1 });
assert.equal(first.created.length, 3);
first.op({ kind: "detach", key: "0" });
first.op({ kind: "adopt", key: "3", type: "odd", index: 3 });
assert.equal(first.created.length, 4);
assert.equal(first.discarded.length, 0);
assert.equal(first.state.live.has("0"), false);
assert.equal(first.state.live.has("3"), true);

const capped = harness(0);
capped.op({ kind: "adopt", key: "0", type: "even", index: 0 }, 0);
capped.op({ kind: "detach", key: "0" }, 0);
assert.equal(capped.discarded.length, 1);

const mixed = harness(2);
mixed.op({ kind: "adopt", key: "1", type: "odd", index: 1 }, 2);
mixed.op({ kind: "detach", key: "1" }, 2);
mixed.op({ kind: "adopt", key: "2", type: "even", index: 2 }, 2);
assert.equal(mixed.created.length, 2);
assert.equal(mixed.state.pools.get("odd")?.length, 1);

console.log("list-binding.test.ts ok");
