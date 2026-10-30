import { createSignal, onCleanup, type Accessor } from "solid-js";
import {
  host,
  listenRefresh,
  onScrollState,
  type NodeHandle,
  type ScrollPhaseName,
} from "@impasto/solid-renderer";
import type { ScrollDriver } from "../../solid-renderer/src/host.ts";

const natives = new WeakMap<ScrollController, ScrollDriver>();

export type ScrollOffset = { x: number; y: number };

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
  scrollTo(x: number, y: number, animate?: { stiffness?: number; damping?: number }): void;
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
  nativeId(): { index: number; generation: number };
};

export function createScrollController(): ScrollController {
  const native = host.scrollController?.();
  if (native === undefined) {
    throw new Error("scrollController is not linked");
  }
  return wrapScroll(native);
}

function wrapScroll(native: ScrollDriver): ScrollController {
  let viewport = 0;
  let content = 0;
  let contentMin = 0;
  let pullMode = 0;
  let threshold = 64;
  let rubber = false;
  let factor = 0.55;
  let parent: ScrollDriver | null = null;
  const [phase, setPhase] = createSignal<ScrollPhaseName>("idle");
  const [overscroll, setOverscroll] = createSignal<ScrollOffset>({ x: 0, y: 0 });
  const offsetListeners = new Set<(offset: ScrollOffset) => void>();
  let stopState = () => {};

  function push(): void {
    native.configure(contentMin, pullMode, threshold, rubber, factor, parent);
  }

  function readOver(): void {
    const sample = native.query(2);
    setOverscroll({ x: sample.x, y: sample.y });
  }

  const controller: ScrollController = {
    node: () => viewport,
    bind(next) {
      stopState();
      viewport = next;
      if (next !== 0) {
        native.bind(next, content);
        stopState = onScrollState(next, (sample) => {
          setPhase(sample.phase);
          setOverscroll({ x: sample.ox, y: sample.oy });
        });
      }
    },
    bindContent(next) {
      content = next;
      if (viewport !== 0) {
        native.bind(viewport, next);
      }
    },
    phase,
    overscroll,
    offset() {
      const sample = native.query(0);
      return { x: sample.x, y: sample.y };
    },
    onOffset(fn) {
      offsetListeners.add(fn);
      return () => {
        offsetListeners.delete(fn);
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
      return { x: sample.x, y: sample.y };
    },
    setLimits(_next) {},
    scrollTo(x, y, animate) {
      if (viewport === 0) {
        return;
      }
      host.scrollTo(viewport, x, y, animate?.stiffness ?? 0, animate?.damping ?? 0);
    },
    fling(vx, vy) {
      if (viewport === 0) {
        return;
      }
      host.fling(viewport, vx, vy);
    },
    stop() {
      if (viewport === 0) {
        return;
      }
      host.stopScroll(viewport);
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
      const taken = native.consume(overflowX, overflowY);
      if (taken) {
        const sample = native.query(0);
        controller.noteOffset({ x: sample.x, y: sample.y });
      }
      return taken;
    },
    nativeId() {
      return native.id();
    },
  };
  natives.set(controller, native);
  onCleanup(() => {
    stopState();
    native.release();
  });
  return controller;
}

export function createScrollLink(
  source: ScrollController,
  target: ScrollController,
  map: (offset: ScrollOffset) => ScrollOffset,
): () => void {
  return source.onOffset((offset) => {
    const next = map(offset);
    const node = target.node();
    if (node === 0) {
      return;
    }
    host.setScrollOffset(node, next.x, next.y);
  });
}

export function refreshListener(controller: ScrollController, fn: () => void): () => void {
  return listenRefresh(controller.nativeId(), fn);
}
