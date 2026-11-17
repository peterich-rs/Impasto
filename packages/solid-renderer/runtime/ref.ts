import { createRenderEffect, onCleanup } from "solid-js";
import type { Node } from "../src/tree.ts";
import type { NodeRef } from "../src/jsx.ts";

/** Point `ref` at `node`, then at null on cleanup or when the callback changes. */
export function bindNodeRef(node: Node, read: () => NodeRef | undefined): void {
  let previous: NodeRef | undefined;
  createRenderEffect(
    () => read(),
    (next) => {
      if (previous !== undefined && previous !== next) previous(null);
      previous = next;
      next?.(node);
    },
  );
  onCleanup(() => {
    previous?.(null);
    previous = undefined;
  });
}
