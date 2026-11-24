import { createMemo, createSignal, createTrackedEffect, onCleanup, type Accessor } from "solid-js";
import { listenScrollThreshold } from "@impasto/solid-renderer";
import type { ScrollController } from "./ScrollController.ts";

export type ScrollThresholdOptions = {
  axis?: "x" | "y" | "overscrollX" | "overscrollY";
  value: number;
  from?: "start" | "end";
  hysteresis?: number;
  enabled?: boolean;
};

function sourceOf(axis: ScrollThresholdOptions["axis"]) {
  switch (axis) {
    case "x": return "scrollX";
    case "overscrollX": return "overscrollX";
    case "overscrollY": return "overscrollY";
    default: return "scrollY";
  }
}

/** Observes crossings without turning each offset into a reactive update. */
export function useScrollThreshold(
  controller: ScrollController,
  options: ScrollThresholdOptions,
): Accessor<boolean> {
  const [active, setActive] = createSignal(false);
  createTrackedEffect(() => {
    const node = controller.node();
    setActive(false);
    if (node === 0 || options.enabled === false) return;
    const source = sourceOf(options.axis);
    const value = options.value;
    const fromEnd = options.from === "end"
      || (options.from === undefined && value < 0 && (source === "scrollX" || source === "scrollY"));
    onCleanup(listenScrollThreshold(node, source, value, fromEnd, options.hysteresis ?? 0, setActive));
  });
  return active;
}

/** Returns the number of sorted vertical breakpoints at or before the offset. */
export function useScrollSegment(
  controller: ScrollController,
  breakpoints: Accessor<readonly number[]>,
): Accessor<number> {
  const [segment, setSegment] = createSignal(0);
  createTrackedEffect(() => {
    const node = controller.node();
    const points = [...breakpoints()];
    if (points.some((point, index) => !Number.isFinite(point) || (index > 0 && point <= points[index - 1]!))) {
      throw new RangeError("Scroll breakpoints must increase and be finite");
    }
    setSegment(0);
    if (node === 0) return;
    const active = new Set<number>();
    for (const point of points) {
      onCleanup(listenScrollThreshold(node, "scrollY", point, false, 0, (next) => {
        if (next) active.add(point);
        else active.delete(point);
        setSegment(active.size);
      }));
    }
  });
  return createMemo(segment);
}
