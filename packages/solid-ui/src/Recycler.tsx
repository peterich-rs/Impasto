import { createRenderEffect, createSignal, onCleanup } from "solid-js";
import { createSlot } from "@impasto/solid-renderer";
import {
  discardRecycler,
  syncRecycler,
  type RecyclerSlot,
  type RecyclerState,
} from "./recycler.ts";

export function Recycler<T>(props: {
  each: readonly T[];
  keyOf: (item: T) => string | number;
  typeOf: (item: T) => string;
  render: (item: () => T, index: () => number) => unknown;
  start?: number;
  poolSize?: number;
  onMounted?: (rows: { key: string; id: number; index: number }[]) => void;
}) {
  const state: RecyclerState<T> = { live: new Map(), pools: new Map() };
  const [parent, setParent] = createSignal(0);

  // The sync callback must not run inside this effect's recomputation.
  // `createTrackedEffect` disposes child owners on every update, and a slot
  // created there would lose the `item` subscription that rewrites its text.
  createRenderEffect(
    () => ({
      entries: props.each.map((item, index) => ({
        key: String(props.keyOf(item)),
        type: props.typeOf(item),
        item,
        index: (props.start ?? 0) + index,
      })),
      limit: props.poolSize ?? 8,
      column: parent(),
    }),
    ({ entries, limit, column }) => {
      syncRecycler({
        state,
        parent: column,
        entries,
        poolSize: limit,
        create(type, item, index) {
          const slot = createSlot(item, index, props.render);
          const wrapped: RecyclerSlot<T> = {
            key: "",
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
            place(nextParent, anchor) {
              slot.place(nextParent, anchor);
            },
          };
          return wrapped;
        },
      });
      if (column === 0) {
        return;
      }
      const rows: { key: string; id: number; index: number }[] = [];
      for (const entry of entries) {
        const slot = state.live.get(entry.key);
        if (slot !== undefined) {
          rows.push({ key: entry.key, id: slot.node, index: entry.index });
        }
      }
      props.onMounted?.(rows);
    },
  );

  onCleanup(() => {
    discardRecycler(state);
  });

  return (
    <column
      ref={(node) => {
        setParent(node.id);
      }}
      gap={0}
    />
  );
}
