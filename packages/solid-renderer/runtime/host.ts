import { createElement, insert, ref, spread } from "./index.ts";
import type { HostProps } from "../src/jsx.ts";
export { Image } from "./image.ts";

function mount(tag: "view" | "stack" | "scroll" | "text", props: HostProps): number {
  const node = createElement(tag);
  spread(node, [props, { ref: undefined }], true);
  if ("children" in props) insert(node, () => props.children);
  if (props.ref) ref(() => props.ref, node);
  return node;
}

/** Raw view. The JSX compiler emits its scene operations directly. */
export function View(props: HostProps): number {
  return mount("view", props);
}

/** Raw single-cell stack. The JSX compiler emits its scene operations directly. */
export function Stack(props: HostProps): number {
  return mount("stack", props);
}

/** Raw scroll viewport. The JSX compiler emits its scene operations directly. */
export function Scroll(props: HostProps & {
  axis?: "vertical" | "horizontal" | "both";
  onScroll?: (x: number, y: number) => void;
}): number {
  return mount("scroll", props);
}

/** Native text leaf for explicit scene values. Rich text components accept JSX children. */
export function Text(props: Omit<HostProps, "children"> & { value?: string }): number {
  return mount("text", props);
}
