import { children, createRenderEffect } from "solid-js";

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
    () => ({
      value: textContent(resolved()),
      fontSize: props.fontSize,
      color: props.color,
      maxLines: props.maxLines,
      ellipsis: props.ellipsis,
      textAlign: props.textAlign,
      weight: props.weight,
      direction: props.direction,
      lineHeight: props.lineHeight,
      family: props.family,
    }),
    (next) => {
      options.replaceText(node, next.value);
      if (next.fontSize !== undefined) {
        options.setProperty(node, "fontSize", next.fontSize);
      }
      if (next.color !== undefined) {
        options.setProperty(node, "color", next.color);
      }
      if (next.maxLines !== undefined) {
        options.setProperty(node, "maxLines", next.maxLines);
      }
      if (next.ellipsis !== undefined) {
        options.setProperty(node, "ellipsis", next.ellipsis);
      }
      if (next.textAlign !== undefined) {
        options.setProperty(node, "textAlign", next.textAlign);
      }
      if (next.weight !== undefined) {
        options.setProperty(node, "weight", next.weight);
      }
      if (next.direction !== undefined) {
        options.setProperty(node, "direction", next.direction);
      }
      if (next.lineHeight !== undefined) {
        options.setProperty(node, "lineHeight", next.lineHeight);
      }
      if (next.family !== undefined) {
        options.setProperty(node, "family", next.family);
      }
    },
  );
  return node;
}
