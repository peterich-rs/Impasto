import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const SURFACE = colorBytes("#FFFFFF");
const HAIR = colorBytes("#E8E8EC");

/** App palette. Brightness follows the platform; the market view is dark-first. */
export const palette = {
  get ink() {
    return INK;
  },
  get muted() {
    return MUTED;
  },
  get surface() {
    return SURFACE;
  },
  get hair() {
    return HAIR;
  },
};

export function bgColor(): [number, number, number, number] {
  return colorBytes("#F5F6F8");
}

export function panelColor(): [number, number, number, number] {
  return SURFACE;
}

export function label(text: string, size: number, color: [number, number, number, number], weight?: number): number {
  const node = host.text(text, size, color);
  if (weight !== undefined) {
    host.setFontWeight(node, weight);
  }
  return node;
}

export type Palette = typeof palette;
export type RGBA = [number, number, number, number];
export type NodeRef = NodeHandle;
export { host };
export { colorBytes };
export function clearChildren(node: number): void {
  for (;;) {
    const first = host.firstChild(node);
    if (first === 0) {
      return;
    }
    host.remove(node, first);
  }
}

/** Real `padding` node with explicit edges; content is appended by the caller. */
export function padNode(top: number, right: number, bottom: number, left: number): number {
  const node = host.padding(0);
  host.setPaddingEdges(node, top, right, bottom, left);
  return node;
}

/** Wraps `child` into a padding node with the given edges. */
export function padded(child: number, top: number, right: number, bottom: number, left: number): number {
  const node = padNode(top, right, bottom, left);
  host.append(node, child);
  return node;
}
