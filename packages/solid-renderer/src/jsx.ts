import type { RecognizerSpec } from "./guard.ts";
import type { StyleProps } from "./style.ts";
import type { Node } from "./tree.ts";

export type NodeRef = (node: Node | null) => void;
export type UIChildren =
  | Node
  | string
  | number
  | boolean
  | null
  | undefined
  | readonly UIChildren[]
  | (() => UIChildren);

type Children = { children?: unknown };
export type ClickEvent = {
  onClick?: () => void;
  onPress?: () => void;
  onRelease?: () => void;
};
export type FocusEvent = {
  focusable?: boolean;
  onFocus?: () => void;
  onBlur?: () => void;
  onKey?: (key: string) => void;
  onText?: (text: string) => void;
  onTextReplaced?: (
    text: string,
    anchor: number,
    caret: number,
    composeFrom: number,
    composeTo: number,
  ) => void;
  onLink?: (link: number) => void;
  onComposing?: (text: string | null) => void;
};
export type DragEvent = {
  onDragBegin?: () => void;
  onDragBy?: (dx: number, dy: number) => void;
  onDragEnd?: (vx: number, vy: number) => void;
};
export type HoverEvent = { onHover?: () => void; onLeave?: () => void };
export type SecondaryEvent = { onSecondaryClick?: () => void };
export type ZoomEvent = { onZoom?: (scale: number) => void };
export type GestureEvent = {
  recognizers?: RecognizerSpec;
  onLongPress?: () => void;
};
export type WheelEvent = { onWheel?: (dx: number, dy: number) => void };
export type ViewEvents = ClickEvent &
  FocusEvent &
  DragEvent &
  HoverEvent &
  SecondaryEvent &
  ZoomEvent &
  GestureEvent &
  WheelEvent;
type ViewProps = Children & StyleProps & ViewEvents & { ref?: NodeRef };

export namespace JSX {
  interface Elements {
    view: ViewProps;
    stack: ViewProps;
    scroll: ViewProps & {
      onScroll?: (x: number, y: number) => void;
      axis?: "vertical" | "horizontal" | "both";
    };
    image: ViewProps & {
      source?: string;
      w?: number;
      h?: number;
      alt?: string;
      mode?: "fill" | "fit" | "crop";
      required?: boolean;
    };
  }
  export type Element = Node;
  export type ElementType = keyof IntrinsicElements | ((props: any) => unknown);
  export type IntrinsicElements = {
    [K in keyof Elements]: Elements[K] & { ref?: NodeRef };
  };
  export interface ElementChildrenAttribute {
    children: unknown;
  }
  export interface IntrinsicAttributes {
    ref?: NodeRef;
  }
}

type NativeElements = JSX.IntrinsicElements;
declare global {
  namespace JSX {
    interface IntrinsicElements extends NativeElements {}
    interface ElementChildrenAttribute {
      children: unknown;
    }
    interface IntrinsicAttributes {
      ref?: NodeRef;
    }
    type Element = Node;
    type ElementType = keyof NativeElements | ((props: any) => unknown);
  }
}
