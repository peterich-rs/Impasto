import { createRoot, createSignal, createTrackedEffect, onCleanup, type Accessor } from "solid-js";
import {
  scene,
  listenRefresh,
  onScrollState,
  type NodeHandle,
  type ScrollPhaseName,
} from "@impasto/solid-renderer";
import type { ScrollDriver } from "../../solid-renderer/src/scene.ts";
const natives = new WeakMap<ScrollController, ScrollDriver>();
export type ScrollOffset = {
  x: number;
  y: number;
};
export type DragStep = {
  ox: number;
  oy: number;
  overX: number;
  overY: number;
  pending: boolean;
};

/**
 * Commands and phase for one scroll. Clamp, drag continuation, and the
 * refresh threshold live on the native class.
 */
export type ScrollController = {
  node(): NodeHandle;
  bind(node: NodeHandle): void;
  bindContent(node: NodeHandle): void;
  readonly phase: Accessor<ScrollPhaseName>;
  readonly overscroll: Accessor<ScrollOffset>;
  offset(): ScrollOffset;
  onOffset(fn: (offset: ScrollOffset) => void): () => void;
  noteOffset(offset: ScrollOffset): void;
  setOverscroll(next: ScrollOffset): void;
  limits(): ScrollOffset;
  setLimits(next: ScrollOffset): void;
  scrollTo(
    x: number,
    y: number,
    animate?: {
      stiffness?: number;
      damping?: number;
    },
  ): void;
  fling(vx: number, vy: number): void;
  stop(): void;
  dragBy(dx: number, dy: number): DragStep;
  finishDrag(taken: boolean): void;
  dragEnd(): void;
  wheel(dx: number, dy: number): void;
  setContentMin(height: number): void;
  setPull(mode: number, threshold: number): void;
  setRubber(rubber: boolean, factor: number): void;
  setParent(parent: ScrollController | null): void;
  consume(overflowX: number, overflowY: number): boolean;
  nativeId(): {
    index: number;
    generation: number;
  };
};
export function createScrollController(): ScrollController {
  const native = scene.scrollController?.();
  if (native === undefined) {
    throw new Error("scrollController is not linked");
  }
  return wrapScroll(native);
}
function wrapScroll(native: ScrollDriver): ScrollController {
  const [viewport, setViewport] = createSignal(0);
  let content = 0;
  let contentMin = 0;
  let pullMode = 0;
  let threshold = 64;
  let rubber = false;
  let factor = 0.55;
  let parent: ScrollDriver | null = null;
  const [phase, setPhase] = createSignal<ScrollPhaseName>("idle");
  const [overscroll, setOverscroll] = createSignal<ScrollOffset>({
    x: 0,
    y: 0,
  });
  const offsetListeners = new Set<(offset: ScrollOffset) => void>();
  let stopState = () => {};
  let released = false;
  let observed = 0;
  function syncOffsets(): void {
    const next = offsetListeners.size === 0 ? 0 : viewport();
    if (next === observed) return;
    if (observed !== 0) scene.observeScrollOffset(observed);
    observed = next;
    if (next !== 0) {
      scene.observeScrollOffset(next, (x, y) => controller.noteOffset({ x, y }));
    }
  }
  function push(): void {
    native.configure(contentMin, pullMode, threshold, rubber, factor, parent);
  }
  function readOver(): void {
    const sample = native.query(2);
    setOverscroll({
      x: sample.x,
      y: sample.y,
    });
  }
  const controller: ScrollController = {
    node: viewport,
    bind(next) {
      if (released) return;
      stopState();
      setViewport(next);
      syncOffsets();
      if (next !== 0) {
        native.bind(next, content);
        stopState = onScrollState(next, (sample) => {
          setPhase(sample.phase);
          setOverscroll({
            x: sample.ox,
            y: sample.oy,
          });
        });
      } else {
        content = 0;
        native.bind(0, 0);
      }
    },
    bindContent(next) {
      content = next;
      if (viewport() !== 0) {
        native.bind(viewport(), next);
      }
    },
    phase,
    overscroll,
    offset() {
      const sample = native.query(0);
      return {
        x: sample.x,
        y: sample.y,
      };
    },
    onOffset(fn) {
      offsetListeners.add(fn);
      syncOffsets();
      return () => {
        offsetListeners.delete(fn);
        syncOffsets();
      };
    },
    noteOffset(offset) {
      for (const fn of offsetListeners) {
        fn(offset);
      }
    },
    setOverscroll(next) {
      setOverscroll(next);
    },
    limits() {
      const sample = native.query(1);
      return {
        x: sample.x,
        y: sample.y,
      };
    },
    setLimits(_next) {},
    scrollTo(x, y, animate) {
      if (viewport() === 0) {
        return;
      }
      scene.scrollTo(
        viewport(),
        x,
        y,
        animate?.stiffness ?? 0,
        animate?.damping ?? 0,
      );
    },
    fling(vx, vy) {
      if (viewport() === 0) {
        return;
      }
      scene.fling(viewport(), vx, vy);
    },
    stop() {
      if (viewport() === 0) {
        return;
      }
      scene.stopScroll(viewport());
    },
    dragBy(dx, dy) {
      const step = native.dragBy(dx, dy);
      if (!step.pending) {
        readOver();
      }
      return step;
    },
    finishDrag(taken) {
      native.finishDrag(taken);
      readOver();
    },
    dragEnd() {
      native.dragEnd();
    },
    wheel(dx, dy) {
      native.wheel(dx, dy);
    },
    setContentMin(height) {
      contentMin = height;
      push();
    },
    setPull(mode, nextThreshold) {
      pullMode = mode;
      threshold = nextThreshold;
      push();
    },
    setRubber(nextRubber, nextFactor) {
      rubber = nextRubber;
      factor = nextFactor;
      push();
    },
    setParent(next) {
      parent = next === null ? null : (natives.get(next) ?? null);
      push();
    },
    consume(overflowX, overflowY) {
      return native.consume(overflowX, overflowY);
    },
    nativeId() {
      return native.id();
    },
  };
  natives.set(controller, native);
  onCleanup(() => {
    released = true;
    stopState();
    offsetListeners.clear();
    syncOffsets();
    native.release();
  });
  return controller;
}
export type ScrollLinkOptions = {
  factor: number;
  offset?: number;
  axis?: "x" | "y" | "both";
};
export function createScrollLink(
  source: ScrollController,
  target: ScrollController,
  map: ((offset: ScrollOffset) => ScrollOffset) | ScrollLinkOptions,
): () => void {
  if (typeof map !== "function") {
    return createRoot((dispose) => {
      createTrackedEffect(() => {
        const from = source.node();
        const to = target.node();
        if (from === 0 || to === 0) return;
        const axis = map.axis ?? "y";
        for (const channel of ["scrollX", "scrollY"] as const) {
          if (axis !== "both" && (axis === "x") !== (channel === "scrollX")) continue;
          const id = scene.bindScrollProp(from, channel, to, channel, map.factor, map.offset ?? 0, "linear");
          onCleanup(() => scene.unbindScrollProp(id));
        }
      });
      return dispose;
    });
  }
  return source.onOffset((offset) => {
    const next = map(offset);
    const node = target.node();
    if (node === 0) {
      return;
    }
    scene.setScrollOffset(node, next.x, next.y);
  });
}
export function refreshListener(
  controller: ScrollController,
  fn: () => void,
): () => void {
  return listenRefresh(controller.nativeId(), fn);
}
