import { createRenderEffect } from "solid-js";

import type { HostNode, HostRendererOptions } from "../src/tree.ts";
import { host } from "./host.ts";

let options: HostRendererOptions;

export function bindImage(next: HostRendererOptions): void {
  options = next;
}

/** An image leaf. `source`, `w`, and `h` update together. */
export function Image(props: { source?: string; w?: number; h?: number; alt?: string }): HostNode {
  const node = options.createElement("image", {
    source: props.source ?? "",
    w: props.w ?? 0,
    h: props.h ?? 0,
  });
  createRenderEffect(
    () => ({
      source: props.source ?? "",
      w: props.w ?? 0,
      h: props.h ?? 0,
      alt: props.alt ?? "",
    }),
    (next) => {
      host.setImage(node.id, next.source, next.w, next.h);
      if (next.alt !== "") {
        host.setSemantics(node.id, 3, next.alt);
      }
    },
  );
  return node;
}
