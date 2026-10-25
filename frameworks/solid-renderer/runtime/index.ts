import { flush } from "solid-js";
import { createRenderer } from "@solidjs/universal";

import type { HostNode } from "../src/tree.ts";
import { hostRendererOptions } from "../src/tree.ts";
import { host } from "./host.ts";
import { bindText } from "./text.ts";
import { bindImage } from "./image.ts";
import { onFrame, runFrames } from "./frames.ts";

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

export { frameListenerCount, onFrame } from "./frames.ts";

/** Asks the host for exactly one more frame after the current one. */
export function requestFrame(): void {
  host.requestFrame();
}

let overlayId = 0;

/** Fill stack painted after the app column. Dialog moves its children here. */
export function overlayParent(): number {
  return overlayId;
}

/** Drops `node` from its parent and forgets the renderer tables for that subtree. */
export function removeNode(node: HostNode): void {
  const parent = scene.getParentNode(node);
  if (parent) {
    scene.removeNode(parent, node);
  }
}

// App-facing core. Constructors return renderer nodes, not raw ids, because
// universal treats a bare number as text.
/** Registers Solid's `flush` and renders `component` into the content column. */
export function boot(component: () => HostNode) {
  const root = scene.createElement("stack", { fill: true });
  const content = scene.createElement("column", { gap: 0, fill: true });
  const overlay = scene.createElement("stack", { fill: true });
  scene.insertNode(root, content);
  scene.insertNode(root, overlay);
  overlayId = overlay.id;
  host.onFrame((targetMs: number, intervalMs: number) => {
    flush();
    runFrames(targetMs, intervalMs);
  });
  render(component, content);
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
  grid(columns: string[], rows: string[], gap: number) {
    return scene.createElement("grid", { columns, rows, gap });
  },
  pointerRegion() {
    return scene.createElement("pointer");
  },
  onFrame(fn: (targetMs: number, intervalMs: number) => void) {
    onFrame(fn);
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
