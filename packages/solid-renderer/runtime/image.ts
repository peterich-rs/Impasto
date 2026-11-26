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

/** Dimensions and density variants extracted from an imported bitmap. */
export type ImageAsset = {
  readonly uri: string;
  readonly width: number;
  readonly height: number;
  readonly scales: readonly number[];
};

export type ImageProps = StyleProps & {
  ref?: NodeRef;
  source?: string | ImageAsset;
  intrinsicSize?: { width: number; height: number };
  alt?: string;
  required?: boolean;
};

/** An image leaf. Author size lives on `style`; `intrinsicSize` is only the measure fallback. */
export function Image(props: ImageProps): Node {
  const node = options.createElement("image", {
    source: typeof props.source === "object" ? props.source.uri : props.source ?? "",
    required: props.required === true,
  });
  bindNodeRef(node, () => props.ref);
  bindStyleTo(options.styles, node, props);
  createRenderEffect(
    () => {
      const asset = typeof props.source === "object" ? props.source : undefined;
      return {
        source: asset?.uri ?? (typeof props.source === "string" ? props.source : ""),
        width: props.intrinsicSize?.width ?? asset?.width,
        height: props.intrinsicSize?.height ?? asset?.height,
        alt: props.alt ?? "",
      };
    },
    (next) => {
      scene.setImage(node, next.source, next.width, next.height);
      if (next.alt !== "") {
        scene.setSemantics(node, 3, next.alt);
      }
    },
  );
  return node;
}
