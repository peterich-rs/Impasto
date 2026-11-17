import { createRenderEffect } from "solid-js";

import type { NodeRef } from "../src/jsx.ts";
import type { StyleProps } from "../src/style.ts";
import type { Node, SceneOptions } from "../src/tree.ts";
import { scene } from "./scene.ts";
import { bindNodeRef } from "./ref.ts";
import { bindStyleTo } from "./style.ts";

let options: SceneOptions;

export function bindImage(next: SceneOptions): void {
  options = next;
}

export type ImageProps = StyleProps & {
  ref?: NodeRef;
  source?: string;
  intrinsicSize?: { width: number; height: number };
  alt?: string;
  required?: boolean;
};

/** An image leaf. Author size lives on `style`; `intrinsicSize` is only the measure fallback. */
export function Image(props: ImageProps): Node {
  const node = options.createElement("image", {
    source: props.source ?? "",
    required: props.required === true,
  });
  bindNodeRef(node, () => props.ref);
  bindStyleTo(options.styles, node, props);
  createRenderEffect(
    () => ({
      source: props.source ?? "",
      width: props.intrinsicSize?.width,
      height: props.intrinsicSize?.height,
      alt: props.alt ?? "",
    }),
    (next) => {
      scene.setImage(node, next.source, next.width, next.height);
      if (next.alt !== "") {
        scene.setSemantics(node, 3, next.alt);
      }
    },
  );
  return node;
}
