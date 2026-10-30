import { createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { createScrollController, type ScrollController } from "./ScrollController.ts";

export type ScrollPhysicsProps = {
  frictionTau?: number;
  overscroll?: "rubber" | "clamp";
  stiffness?: number;
  damping?: number;
  factor?: number;
  snapX?: { interval: number; align?: "start" | "center" | "end" };
  snapY?: { interval: number; align?: "start" | "center" | "end" };
  chain?: boolean;
  /** `content` yields at the edge. `always` keeps the drag and can rubber-band. `never` passes the drag to a parent. */
  accept?: "content" | "always" | "never";
};

function acceptCode(accept: ScrollPhysicsProps["accept"]): number {
  if (accept === "always") {
    return 1;
  }
  if (accept === "never") {
    return 2;
  }
  return 0;
}

/**
 * A scroller on one axis. The default is vertical, and `accept` is always, so
 * an edge drag stays here and can rubber-band. Pass `horizontal` for a pager.
 * `content` yields at the edge. Drag, wheel, and clamp run on the scroll
 * controller. After release, `core` coasts on the declared axis.
 */
export function ScrollArea(props: {
  children?: unknown;
  axis?: "vertical" | "horizontal" | "both";
  onOffset?: (offset: { x: number; y: number }, viewport: number) => void;
  viewportRef?: (viewport: number) => void;
  contentRef?: (content: number) => void;
  onDragEnd?: () => void;
  initialOffset?: { x: number; y: number };
  /** Minimum content height. Keeps the viewport scrollable when row layout lags the window. */
  contentMin?: number;
  controller?: ScrollController;
  physics?: ScrollPhysicsProps;
  /** Remainder after this scroll clamped a drag it already owns. Return true if a parent took it. */
  onDragOverflow?: (overflowX: number, overflowY: number) => boolean;
}): NodeHandle {
  const ctrl = props.controller ?? createScrollController();
  let viewport = 0;
  let content = 0;

  function applyContentMin(): void {
    const min = props.contentMin ?? 0;
    ctrl.setContentMin(min);
    if (content !== 0 && Number.isFinite(min) && min > 0) {
      host.setMinSize(content, 0, min);
    }
  }

  function applyPhysics(node: number): void {
    const physics = props.physics;
    const align = (name: "start" | "center" | "end" | undefined) =>
      name === "center" ? 1 : name === "end" ? 2 : 0;
    host.setScrollPhysics(
      node,
      physics?.frictionTau ?? 180,
      physics?.overscroll === "clamp" ? 1 : 0,
      physics?.stiffness ?? 170,
      physics?.damping ?? 26,
      physics?.factor ?? 0.55,
      physics?.snapX?.interval ?? 0,
      align(physics?.snapX?.align),
      physics?.snapY?.interval ?? 0,
      align(physics?.snapY?.align),
      acceptCode(physics?.accept ?? "always"),
    );
    if (physics?.chain !== undefined) {
      host.setScrollChain(node, physics.chain);
    }
    ctrl.setRubber(physics?.overscroll === "rubber", physics?.factor ?? 0.55);
  }

  createTrackedEffect(() => {
    applyContentMin();
  });

  return (
    <scroll
      axis={props.axis ?? "vertical"}
      fill
      ref={(node) => {
        viewport = node.id;
        ctrl.bind(node.id);
        applyPhysics(node.id);
        const initial = props.initialOffset;
        if (initial !== undefined && (initial.x !== 0 || initial.y !== 0)) {
          host.setScrollOffset(viewport, initial.x, initial.y);
          ctrl.bind(node.id);
        }
        props.viewportRef?.(node.id);
      }}
      onDragBy={(dx, dy) => {
        const step = ctrl.dragBy(dx, dy);
        if (!step.pending) {
          return;
        }
        const taken = props.onDragOverflow?.(step.ox, step.oy) === true;
        ctrl.finishDrag(taken);
      }}
      onDragEnd={() => {
        ctrl.dragEnd();
        props.onDragEnd?.();
      }}
      onWheel={(dx, dy) => {
        ctrl.wheel(dx, dy);
      }}
    >
      <column
        ref={(node) => {
          content = node.id;
          ctrl.bindContent(node.id);
          applyContentMin();
          props.contentRef?.(node.id);
        }}
      >
        {props.children}
      </column>
    </scroll>
  );
}
