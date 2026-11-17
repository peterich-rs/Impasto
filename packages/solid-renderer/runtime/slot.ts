import { createRoot, createSignal, runWithOwner, untrack } from "solid-js";

import type { SceneModule, NodeHandle } from "../src/scene.ts";
import type { UIChildren } from "../src/jsx.ts";
import type { Node } from "../src/tree.ts";

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
  createElement(tag: string, staticProps?: Record<string, unknown>): Node;
  insert(parent: Node, accessor: () => UIChildren): void;
  forget(id: number): void;
  scene: SceneModule;
};

export function bindSlot(deps: SlotDeps) {
  return function createSlot<T>(
    initial: T,
    at: number,
    render: (item: () => T, index: () => number) => UIChildren,
  ): Slot<T> {
    let root!: Node;
    let setItem!: (value: T) => void;
    let setIndex!: (value: number) => void;
    // Detach from the caller. A tracked effect disposes its child owners on
    // every update, which would drop the subscription that rewrites this shell.
    const dispose = runWithOwner(null, () =>
      createRoot((done) => {
        const [state, writeItem] = createSignal({ value: initial });
        const item = () => state().value;
        const [index, writeIndex] = createSignal(at);
        setItem = (value) => {
          writeItem({ value });
        };
        setIndex = writeIndex;
        root = deps.createElement("view", { style: { gap: 0 } });
        // `untrack` keeps `item` reads inside `render` from rebuilding this shell.
        deps.insert(root, () => untrack(() => render(item, index)));
        return done;
      }),
    );
    return {
      get node() {
        return root;
      },
      set(next, index) {
        setItem(next);
        setIndex(index);
      },
      attach(parent, anchor) {
        deps.scene.insertNode(parent, root, anchor ?? 0);
      },
      detach() {
        const parent = deps.scene.parent(root);
        if (parent !== 0) {
          deps.scene.detach(parent, root);
        }
      },
      place(parent, anchor = 0) {
        if (
          deps.scene.parent(root) === parent &&
          deps.scene.nextSibling(root) === anchor
        ) {
          return;
        }
        deps.scene.insertNode(parent, root, anchor);
      },
      discard() {
        const parent = deps.scene.parent(root);
        if (parent !== 0) {
          deps.scene.detach(parent, root);
        }
        dispose();
        deps.forget(root);
        deps.scene.discard(root);
      },
    };
  };
}
