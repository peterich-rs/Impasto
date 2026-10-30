import { children, createRenderEffect, createTrackedEffect } from "solid-js";

import type { HostNode, HostRendererOptions } from "../src/tree.ts";

function textContent(value: unknown): string {
  if (typeof value === "function") {
    return textContent((value as () => unknown)());
  }
  if (Array.isArray(value)) {
    return value.map((item) => textContent(item)).join("");
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number") {
    return String(value);
  }
  return "";
}

let options: HostRendererOptions;

export function bindText(next: HostRendererOptions): void {
  options = next;
}

/** A text leaf. Strings cannot be host children: core text nodes are not parents. */
export function Text(props: {
  fontSize?: number;
  color?: string;
  maxLines?: number;
  ellipsis?: string;
  softWrap?: boolean;
  overflow?: "clip" | "ellipsis" | "visible";
  textAlign?: "left" | "center" | "right";
  weight?: number;
  direction?: "ltr" | "rtl";
  lineHeight?: number;
  family?: string;
  children?: unknown;
}): HostNode {
  const resolved = children(() => props.children as never);
  const node = options.createTextNode("");
  createRenderEffect(
    () => textContent(resolved()),
    (value) => {
      options.replaceText(node, value);
    },
  );
  createRenderEffect(
    () => props.color,
    (color) => {
      if (color !== undefined) {
        options.setProperty(node, "color", color);
      }
    },
  );
  createTrackedEffect(() => {
    if (props.fontSize !== undefined) {
      options.setProperty(node, "fontSize", props.fontSize);
    }
    if (props.maxLines !== undefined) {
      options.setProperty(node, "maxLines", props.maxLines);
    }
    if (props.ellipsis !== undefined) {
      options.setProperty(node, "ellipsis", props.ellipsis);
    }
    if (props.softWrap !== undefined) {
      options.setProperty(node, "softWrap", props.softWrap);
    }
    if (props.overflow !== undefined) {
      options.setProperty(node, "overflow", props.overflow);
    }
    if (props.textAlign !== undefined) {
      options.setProperty(node, "textAlign", props.textAlign);
    }
    if (props.weight !== undefined) {
      options.setProperty(node, "weight", props.weight);
    }
    if (props.direction !== undefined) {
      options.setProperty(node, "direction", props.direction);
    }
    if (props.lineHeight !== undefined) {
      options.setProperty(node, "lineHeight", props.lineHeight);
    }
    if (props.family !== undefined) {
      options.setProperty(node, "family", props.family);
    }
  });
  return node;
}
