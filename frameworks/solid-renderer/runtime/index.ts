import { flush } from "solid-js";
import { createRenderer } from "@solidjs/universal";

import type { HostNode } from "../src/tree.ts";
import { hostRendererOptions } from "../src/tree.ts";
import { host } from "./host.ts";
import { bindText } from "./text.ts";
import { bindImage } from "./image.ts";

const scene = hostRendererOptions(host);
bindText(scene);
bindImage(scene);
const renderer = createRenderer<HostNode>(scene);

export const {
  render,
  insert,
  spread,
  effect,
  memo,
  createComponent,
  setProp,
  mergeProps,
  applyRef,
  ref,
  createElement,
  createTextNode,
  insertNode,
} = renderer;

export { Text } from "./text.ts";
export { Image } from "./image.ts";
export { host } from "./host.ts";
export type { HostNode as NodeHandle } from "../src/tree.ts";

const ticks: Array<(timeMs: number) => void> = [];

/** Runs after Solid flushes, once per frame the host delivers. */
export function onFrame(fn: (timeMs: number) => void): void {
  ticks.push(fn);
}

/** Asks the host for exactly one more frame after the current one. */
export function requestFrame(): void {
  host.requestFrame();
}

// App-facing core. Constructors return renderer nodes, not raw ids, because
// universal treats a bare number as text.
/** Registers Solid's `flush` and renders `component` into a root column. */
export function boot(component: () => HostNode) {
  const root = scene.createElement("column", { gap: 0, fill: true });
  host.onFrame((timeMs: number) => {
    flush();
    for (const fn of ticks) {
      fn(timeMs);
    }
  });
  render(component, root);
}

export const core = {
  column(gap: number) {
    return scene.createElement("column", { gap });
  },
  row(gap: number) {
    return scene.createElement("row", { gap });
  },
  stack() {
    return scene.createElement("stack");
  },
  padding(edges: number) {
    return scene.createElement("padding", { edges });
  },
  pointerRegion() {
    return scene.createElement("pointer");
  },
  onFrame(fn: (timeMs: number) => void) {
    ticks.push(fn);
  },
};

// builtIns import these names from moduleName. rc.17 calls the boundary Errored.
export { Errored, Errored as ErrorBoundary, For, Match, Show, Switch } from "solid-js";

export function Dynamic(props: {
  component?: ((props: Record<string, unknown>) => HostNode) | null;
}): HostNode {
  const component = props.component;
  if (typeof component !== "function") {
    return scene.createTextNode("");
  }
  const { component: _component, ...rest } = props;
  return createComponent(component, rest as never);
}
