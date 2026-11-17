import { bindStyleTo } from "./style.ts";
import { bindNodeRef } from "./ref.ts";
import type { NodeRef } from "../src/jsx.ts";
import type { StyleProps } from "../src/style.ts";
import { children, createRenderEffect } from "solid-js";

import type { Node, SceneOptions } from "../src/tree.ts";

export type TextGradient = {
  kind: "linear" | "radial" | "sweep";
  from?: [number, number];
  to?: [number, number];
  center?: [number, number];
  radius?: number;
  startAngle?: number;
  endAngle?: number;
  stops: { offset: number; color: string | [number, number, number, number] }[];
};

export type TextRun = {
  text: string;
  color?: string | [number, number, number, number];
  weight?: number;
  family?: string;
  underline?: boolean;
  lineThrough?: boolean;
  gradient?: TextGradient;
};

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

let options: SceneOptions;

export function bindText(next: SceneOptions): void {
  options = next;
}

export type TextProps = StyleProps & {
  ref?: NodeRef;
  runs?: TextRun[];
  children?: unknown;
};

/** A text leaf. Strings cannot be scene children: core text nodes are not parents. */
export function Text(props: TextProps): Node {
  const resolved = children(() => props.children as never);
  const node = options.createTextNode("");
  bindNodeRef(node, () => props.ref);
  createRenderEffect(
    () => props.runs,
    (runs) => {
      if (runs !== undefined) {
        options.setProperty(node, "runs", runs);
      }
    },
  );
  createRenderEffect(
    () => (props.runs === undefined ? textContent(resolved()) : undefined),
    (value) => {
      if (value !== undefined) {
        options.replaceText(node, value);
      }
    },
  );
  bindStyleTo(options.styles, node, props);
  return node;
}
