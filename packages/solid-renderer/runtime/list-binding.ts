import type { ListDriver } from "../src/host.ts";
import type { HostNode } from "../src/tree.ts";
import { listenList, type ListChrome, type ListOp } from "./driver.ts";
import type { Slot } from "./slot.ts";

export type ListBindingSlot<T> = {
  key: string;
  type: string;
  node: number;
  set(item: T, index: number): void;
  detach(): void;
  discard(): void;
  place(parent: number, anchor: number): void;
};

export type ListBindingState<T> = {
  live: Map<string, ListBindingSlot<T>>;
  pools: Map<string, ListBindingSlot<T>[]>;
};

type ApplyHooks<T> = {
  parent(): number;
  poolSize(): number;
  itemAt(index: number): T;
  create(type: string, item: T, index: number, key: string): ListBindingSlot<T>;
  note(key: string, node: number): void;
};

/** Applies one driver op to the language-side slot pool. */
export function applyListOp<T>(state: ListBindingState<T>, op: ListOp, hooks: ApplyHooks<T>): void {
  if (op.kind === "detach") {
    const slot = state.live.get(op.key);
    if (slot === undefined) {
      return;
    }
    state.live.delete(op.key);
    hooks.note(op.key, 0);
    park(state, slot, hooks.poolSize());
    return;
  }
  if (op.kind === "adopt") {
    const item = hooks.itemAt(op.index);
    let slot = state.live.get(op.key);
    if (slot === undefined || slot.type !== op.type) {
      if (slot !== undefined) {
        state.live.delete(op.key);
        hooks.note(op.key, 0);
        park(state, slot, hooks.poolSize());
      }
      slot = take(state, op.type, item, op.index, op.key, hooks.create);
      state.live.set(op.key, slot);
    } else {
      slot.set(item, op.index);
    }
    hooks.note(op.key, slot.node);
    return;
  }
  if (op.kind === "set") {
    const slot = state.live.get(op.key);
    if (slot === undefined) {
      return;
    }
    slot.set(hooks.itemAt(op.index), op.index);
    hooks.note(op.key, slot.node);
    return;
  }
  const slot = state.live.get(op.key);
  const parent = hooks.parent();
  if (slot === undefined || parent === 0) {
    return;
  }
  const before = op.before.length === 0 ? 0 : (state.live.get(op.before)?.node ?? 0);
  slot.place(parent, before);
}

function park<T>(state: ListBindingState<T>, slot: ListBindingSlot<T>, poolSize: number): void {
  slot.detach();
  const pooled = state.pools.get(slot.type) ?? [];
  pooled.push(slot);
  state.pools.set(slot.type, pooled);
  while (pooled.length > poolSize) {
    pooled.shift()?.discard();
  }
}

function take<T>(
  state: ListBindingState<T>,
  type: string,
  item: T,
  index: number,
  key: string,
  create: ApplyHooks<T>["create"],
): ListBindingSlot<T> {
  const pooled = state.pools.get(type);
  const slot = pooled?.pop();
  if (slot === undefined) {
    return create(type, item, index, key);
  }
  slot.key = key;
  slot.set(item, index);
  return slot;
}

export function bindListRenderer<T>(
  driver: ListDriver,
  options: {
    parent(): number;
    poolSize(): number;
    itemAt(index: number): T;
    render(item: () => T, index: () => number): HostNode;
    createSlot(
      initial: T,
      at: number,
      render: (item: () => T, index: () => number) => HostNode,
    ): Slot<T>;
    onChrome(chrome: ListChrome): void;
    onEnd(): void;
  },
): () => void {
  const state: ListBindingState<T> = { live: new Map(), pools: new Map() };
  const id = driver.id();
  return listenList({
    id,
    onOp(op) {
      applyListOp(state, op, {
        parent: options.parent,
        poolSize: options.poolSize,
        itemAt: options.itemAt,
        note(key, node) {
          driver.noteRow(key, node);
        },
        create(type, item, index, key) {
          const slot = options.createSlot(item, index, options.render);
          return {
            key,
            type,
            get node() {
              return slot.node;
            },
            set(next, at) {
              slot.set(next, at);
            },
            detach() {
              slot.detach();
            },
            discard() {
              slot.discard();
            },
            place(parent, anchor) {
              slot.place(parent, anchor);
            },
          };
        },
      });
    },
    onChrome: options.onChrome,
    onEnd: options.onEnd,
  });
}
