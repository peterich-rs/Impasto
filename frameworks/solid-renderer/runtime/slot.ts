import { createRoot, createSignal, runWithOwner, untrack } from "solid-js";

import type { CoreHost, NodeHandle } from "../src/host.ts";
import type { HostNode } from "../src/tree.ts";

/** A shell that survives leaving its parent. `render` runs once. */
export type Slot<T> = {
  readonly node: NodeHandle;
  set(item: T, index: number): void;
  attach(parent: NodeHandle, anchor?: NodeHandle): void;
  detach(): void;
  place(parent: NodeHandle, anchor?: NodeHandle): void;
  discard(): void;
};

export type SlotDeps = {
  createElement(tag: string, staticProps?: Record<string, unknown>): HostNode;
  insert(parent: HostNode, accessor: () => HostNode): void;
  forget(id: number): void;
  host: CoreHost;
};

export function bindSlot(deps: SlotDeps) {
  return function createSlot<T>(
    initial: T,
    at: number,
    render: (item: () => T, index: () => number) => HostNode,
  ): Slot<T> {
    let root!: HostNode;
    let setItem!: (value: T) => void;
    let setIndex!: (value: number) => void;
    // Detach from the caller. A tracked effect disposes its child owners on
    // every update, which would drop the subscription that rewrites this shell.
    const dispose = runWithOwner(null, () =>
      createRoot((done) => {
        const [item, writeItem] = createSignal(initial);
        const [index, writeIndex] = createSignal(at);
        setItem = writeItem;
        setIndex = writeIndex;
        root = deps.createElement("column", { gap: 0 });
        // `untrack` keeps `item` reads inside `render` from rebuilding this shell.
        deps.insert(root, () => untrack(() => render(item, index)));
        return done;
      }),
    );
    return {
      get node() {
        return root.id;
      },
      set(next, index) {
        setItem(next);
        setIndex(index);
      },
      attach(parent, anchor) {
        deps.host.insert(parent, root.id, anchor ?? 0);
      },
      detach() {
        const parent = deps.host.parent(root.id);
        if (parent !== 0) {
          deps.host.detach(parent, root.id);
        }
      },
      place(parent, anchor = 0) {
        if (deps.host.parent(root.id) === parent && deps.host.nextSibling(root.id) === anchor) {
          return;
        }
        deps.host.insert(parent, root.id, anchor);
      },
      discard() {
        const parent = deps.host.parent(root.id);
        if (parent !== 0) {
          deps.host.detach(parent, root.id);
        }
        dispose();
        deps.forget(root.id);
        deps.host.discard(root.id);
      },
    };
  };
}
