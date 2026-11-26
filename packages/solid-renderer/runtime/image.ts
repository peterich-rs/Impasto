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

type ImageSource = [ImageProps["source"], ImageProps["intrinsicSize"], ImageProps["alt"]];

export function bindImageSource(node: Node, read: () => ImageSource, required?: boolean): void {
  let labeled = false;
  createRenderEffect(
    () => {
      const [source, size, alt] = read();
      const asset = typeof source === "object" ? source : undefined;
      return {
        uri: asset?.uri ?? (typeof source === "string" ? source : ""),
        width: size?.width ?? asset?.width,
        height: size?.height ?? asset?.height,
        alt,
      };
    },
    ({ uri, width, height, alt }) => {
      scene.setImage(node, uri, width, height, required);
      required = undefined;
      if (alt || labeled) scene.setSemantics(node, 3, alt || undefined);
      labeled = !!alt;
    },
  );
}

/** An image leaf. Author size lives on `style`; `intrinsicSize` is only the measure fallback. */
export function Image(props: ImageProps): Node {
  const node = options.createElement("image", {
    source: typeof props.source === "object" ? props.source.uri : props.source ?? "",
    required: props.required === true,
  });
  bindNodeRef(node, () => props.ref);
  bindStyleTo(options.styles, node, props);
  bindImageSource(node, () => [props.source, props.intrinsicSize, props.alt]);
  return node;
}
