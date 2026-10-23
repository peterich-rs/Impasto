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
type Drag = {
  onDragBegin?: () => void;
  onDragBy?: (dx: number, dy: number) => void;
  onDragEnd?: (vx: number, vy: number) => void;
};

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      column: Children & Grow & Flex & { gap?: number };
      row: Children & Grow & Flex & { gap?: number };
      stack: Children & Grow;
      padding: Children & Grow & { edges?: number };
      pointer: Children & Click & Focusable & Grow;
      sized: { w?: number; h?: number };
      clip: Children & Grow & { radius?: number };
      opacity: Children & Grow & { alpha?: number };
      scroll: Children & Drag;
      transform: Children & { x?: number; y?: number; rotate?: number; scale?: number };
      image: { source?: string; w?: number; h?: number };
      decorated: Children & Grow & { color?: string; radius?: number };
    }
    type Element = HostNode;
  }
}

export {};
