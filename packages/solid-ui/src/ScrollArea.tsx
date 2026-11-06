import { createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@impasto/solid-renderer";
import { createScrollController, type ScrollController } from "./ScrollController.ts";

/** One scroll's motion. Presets are values of this type, not platform names. */
export type ScrollPhysics = {
  decay: { tau: number } | { rate: number };
  band: { linear: number } | { curve: number };
  overscroll: "rubber" | "clamp";
  spring: { stiffness: number; damping: number };
  snap: { stiffness: number; damping: number };
  snapX?: { interval: number; align?: "start" | "center" | "end" };
  snapY?: { interval: number; align?: "start" | "center" | "end" };
  chain?: boolean;
  /** `content` yields at the edge. `always` keeps the drag and can rubber-band. `never` passes the drag to a parent. */
  accept?: "content" | "always" | "never";
};

/** Asymptotic band, 500ms coast, 80ms return. `accept` is left unset. */
export const bouncing: ScrollPhysics = {
  decay: { tau: 500 },
  band: { curve: 0.55 },
  overscroll: "rubber",
  spring: { stiffness: 2500, damping: 100 },
  snap: { stiffness: 220, damping: 30 },
};

/** Offset stops at the limit. Overscroll stays zero. */
export const clamping: ScrollPhysics = {
  ...bouncing,
  overscroll: "clamp",
};

function acceptCode(accept: ScrollPhysics["accept"]): number {
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
  onDragBy?: () => void;
  initialOffset?: { x: number; y: number };
  /** Minimum content height. Keeps the viewport scrollable when row layout lags the window. */
  contentMin?: number;
  controller?: ScrollController;
  physics?: ScrollPhysics;
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
    const physics = props.physics ?? bouncing;
    const align = (name: "start" | "center" | "end" | undefined) =>
      name === "center" ? 1 : name === "end" ? 2 : 0;
    const decayKind = "rate" in physics.decay ? 1 : 0;
    const decayValue = "rate" in physics.decay ? physics.decay.rate : physics.decay.tau;
    const bandKind = "curve" in physics.band ? 1 : 0;
    const bandValue = "curve" in physics.band ? physics.band.curve : physics.band.linear;
    host.setScrollPhysics(
      node,
      decayValue,
      physics.overscroll === "clamp" ? 1 : 0,
      physics.spring.stiffness,
      physics.spring.damping,
      bandValue,
      physics.snapX?.interval ?? 0,
      align(physics.snapX?.align),
      physics.snapY?.interval ?? 0,
      align(physics.snapY?.align),
      acceptCode(physics.accept ?? "always"),
      decayKind,
      physics.snap.stiffness,
      physics.snap.damping,
      bandKind,
    );
    if (physics.chain !== undefined) {
      host.setScrollChain(node, physics.chain);
    }
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
      onDragBy={props.onDragBy === undefined ? undefined : () => props.onDragBy?.()}
      onDragEnd={props.onDragEnd === undefined ? undefined : () => props.onDragEnd?.()}
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
