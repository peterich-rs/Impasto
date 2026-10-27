import type { AxisSize, GridLine, InsetEdge } from "./layout-values.ts";
import type { HostNode } from "./tree.ts";

type Children = { children?: unknown };
type Click = { onClick?: () => void; onPress?: () => void; onRelease?: () => void };
type Focusable = {
  focusable?: boolean;
  onFocus?: () => void;
  onBlur?: () => void;
  onKey?: (key: string) => void;
  onText?: (text: string) => void;
  onTextReplaced?: (text: string) => void;
  onLink?: (link: number) => void;
  onComposing?: (text: string | null) => void;
};
type Grow = { grow?: number };
type Flex = {
  justify?: "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly";
  cross?: "start" | "center" | "end" | "stretch" | "baseline";
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
type Zoom = { onZoom?: (scale: number) => void };
type Wheel = { onWheel?: (dx: number, dy: number) => void };
type Place = {
  aspect?: number | null;
  absolute?: boolean;
  insetTop?: InsetEdge;
  insetRight?: InsetEdge;
  insetBottom?: InsetEdge;
  insetLeft?: InsetEdge;
  columnStart?: GridLine;
  columnEnd?: GridLine;
  rowStart?: GridLine;
  rowEnd?: GridLine;
};
type Box = {
  margin?: number;
  min?: { w?: number; h?: number };
  max?: { w?: number; h?: number };
  shrink?: number;
  basis?: number;
  alignSelf?: "start" | "center" | "end";
} & Place;
type Tracks = { columns?: string[]; rows?: string[]; gap?: number };
type PaddingEdges = { edges?: number; top?: number; right?: number; bottom?: number; left?: number };

declare module "solid-js" {
  namespace JSX {
    interface IntrinsicElements {
      column: Children & Grow & Flex & Box & { gap?: number };
      row: Children & Grow & Flex & Box & { gap?: number; w?: AxisSize; h?: AxisSize };
      stack: Children & Grow & Box & Fill;
      align: Children & Grow & Box & Fill & { align?: "start" | "center" | "end" };
      grid: Children & Grow & Flex & Box & Tracks;
      padding: Children & Grow & Fill & Box & PaddingEdges;
      pointer: Children & Click & Focusable & Grow & Hover & Secondary & Zoom & Box & Fill;
      sized: { w?: AxisSize; h?: AxisSize } & Place;
      clip: Children & Grow & Box & { radius?: number };
      opacity: Children & Grow & Fill & { alpha?: number };
      scroll: Children & Drag & Wheel & Box & { onScroll?: (x: number, y: number) => void };
      transform: Children & Zoom & Fill & { x?: number; y?: number; rotate?: number; scale?: number };
      image: {
        source?: string;
        w?: number;
        h?: number;
        alt?: string;
        mode?: "fill" | "fit" | "crop";
        required?: boolean;
      };
      decorated: Children & Grow & Fill & { color?: string; radius?: number };
    }
    type Element = HostNode;
  }
}

export {};
