import { createTrackedEffect, onCleanup } from "solid-js";
import {
  bindNodeRef,
  bindStyle,
  onLayout,
  scene,
  type NodeHandle,
} from "@impasto/solid-renderer";
import type { ScrollAreaProps } from "./props.ts";
import {
  createScrollController,
  type ScrollController,
} from "./ScrollController.ts";

/** One scroll's motion. Presets are values of this type, not platform names. */
export type ScrollPhysics = {
  decay:
    | {
        tau: number;
      }
    | {
        rate: number;
      };
  band:
    | {
        linear: number;
      }
    | {
        curve: number;
      };
  overscroll: "rubber" | "clamp";
  spring: {
    stiffness: number;
    damping: number;
  };
  snap: {
    stiffness: number;
    damping: number;
  };
  snapX?: {
    interval: number;
    align?: "start" | "center" | "end";
  };
  snapY?: {
    interval: number;
    align?: "start" | "center" | "end";
  };
  chain?: boolean;
  /** `content` yields at the edge. `always` keeps the drag and can rubber-band. `never` passes the drag to a parent. */
  accept?: "content" | "always" | "never";
};

/** Asymptotic band, 500ms coast, 80ms return. `accept` is left unset. */
export const bouncing: ScrollPhysics = {
  decay: {
    tau: 500,
  },
  band: {
    curve: 0.55,
  },
  overscroll: "rubber",
  spring: {
    stiffness: 2500,
    damping: 100,
  },
  snap: {
    stiffness: 220,
    damping: 30,
  },
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
export function ScrollArea(props: ScrollAreaProps): NodeHandle {
  const ctrl = props.controller ?? createScrollController();
  let viewport = 0;
  let content = 0;
  let minSource = 0;
  let stopRestoring = () => {};
  function applyContentMin(): void {
    const min = props.contentMin ?? 0;
    ctrl.setContentMin(min);
    if (content === 0) {
      return;
    }
    if (minSource === 0) {
      minSource = scene.createStyleSource(content, "component");
    }
    scene.setStyleSource(
      minSource,
      Number.isFinite(min) && min > 0 ? { minWidth: 0, minHeight: min } : null,
    );
  }
  onCleanup(() => {
    stopRestoring();
    if (ctrl.node() === viewport && viewport !== 0) ctrl.bind(0);
    if (minSource !== 0) scene.removeStyleSource(minSource);
  });
  function applyPhysics(node: number): void {
    const physics = props.physics ?? bouncing;
    const align = (name: "start" | "center" | "end" | undefined) =>
      name === "center" ? 1 : name === "end" ? 2 : 0;
    const decayKind = "rate" in physics.decay ? 1 : 0;
    const decayValue =
      "rate" in physics.decay ? physics.decay.rate : physics.decay.tau;
    const bandKind = "curve" in physics.band ? 1 : 0;
    const bandValue =
      "curve" in physics.band ? physics.band.curve : physics.band.linear;
    scene.setScrollPhysics(
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
      scene.setScrollChain(node, physics.chain);
    }
  }
  createTrackedEffect(() => {
    applyContentMin();
  });
  return (
    <scroll
      axis={props.axis ?? "vertical"}
      ref={(node) => {
        if (node === null) return;
        viewport = node;
        ctrl.bind(node);
        applyPhysics(node);
        bindStyle(node, {
          defaults: {
            width: "100%",
            height: "100%",
            flex: 1,
            flexShrink: 1,
            minWidth: 0,
            minHeight: 0,
          },
          get class() {
            return props.class;
          },
          get classList() {
            return props.classList;
          },
          get style() {
            return props.style;
          },
        });
        bindNodeRef(node, () => props.ref);
        bindNodeRef(node, () => props.viewportRef);
        createTrackedEffect(() => {
          const listener = props.onOffset;
          if (listener) onCleanup(ctrl.onOffset((offset) => listener(offset, node)));
        });
        const initial = props.initialOffset;
        stopRestoring();
        if (initial !== undefined && (initial.x !== 0 || initial.y !== 0)) {
          stopRestoring = onLayout(() => {
            if (content === 0 || viewport !== node) return;
            stopRestoring();
            scene.setScrollOffset(node, initial.x, initial.y);
          });
        }
      }}
      onDragBy={
        props.onDragBy === undefined ? undefined : () => props.onDragBy?.()
      }
      onDragEnd={
        props.onDragEnd === undefined ? undefined : () => props.onDragEnd?.()
      }
    >
      <view
        ref={(node) => {
          if (node === null) return;
          content = node;
          ctrl.bindContent(node);
          bindStyle(node, {
            get class() {
              return props.contentProps?.class;
            },
            get classList() {
              return props.contentProps?.classList;
            },
            get style() {
              return props.contentProps?.style;
            },
          });
          applyContentMin();
          bindNodeRef(node, () => props.contentRef);
        }}
      >
        {props.children}
      </view>
    </scroll>
  );
}
