import type { HostNode } from "./tree.ts";

type Children = { children?: unknown };
type Click = { onClick?: () => void; onPress?: () => void; onRelease?: () => void };
type Focusable = {
  focusable?: boolean;
  onFocus?: () => void;
  onBlur?: () => void;
  onKey?: (key: string) => void;
  onText?: (text: string) => void;
  onComposing?: (text: string | null) => void;
};
type Grow = { grow?: number };
type Flex = {
  justify?: "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly";
  wrap?: boolean;
  fill?: boolean;
};
type Fill = { fill?: boolean };
type Drag = {
  onDragBegin?: () => void;
  onDragBy?: (dx: number, dy: number) => void;
  onDragEnd?: (vx: number, vy: number) => void;
};
type Hover = { onHover?: () => void; onLeave?: () => void };
type Secondary = { onSecondaryClick?: () => void };
type Wheel = { onWheel?: (dx: number, dy: number) => void };
type Box = {
  margin?: number;
  min?: { w?: number; h?: number };
  max?: { w?: number; h?: number };
  shrink?: number;
  basis?: number;
  alignSelf?: "start" | "center" | "end";
};
type Tracks = { columns?: string[]; rows?: string[]; gap?: number };
type PaddingEdges = { edges?: number; top?: number; right?: number; bottom?: number; left?: number };

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      column: Children & Grow & Flex & Box & { gap?: number };
      row: Children & Grow & Flex & Box & { gap?: number };
      stack: Children & Grow & Box & Fill;
      grid: Children & Grow & Flex & Box & Tracks;
      padding: Children & Grow & Box & PaddingEdges;
      pointer: Children & Click & Focusable & Grow & Hover & Secondary & Box & Fill;
      sized: { w?: number; h?: number };
      clip: Children & Grow & { radius?: number };
      opacity: Children & Grow & { alpha?: number };
      scroll: Children & Drag & Wheel & Box;
      transform: Children & { x?: number; y?: number; rotate?: number; scale?: number };
      image: { source?: string; w?: number; h?: number };
      decorated: Children & Grow & Fill & { color?: string; radius?: number };
    }
    type Element = HostNode;
  }
}

export {};
