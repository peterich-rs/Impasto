export type ScrollPhaseName = "idle" | "dragging" | "coasting" | "settling";

export type ScrollSample = {
  phase: ScrollPhaseName;
  vx: number;
  vy: number;
  ox: number;
  oy: number;
};

const listeners = new Map<number, Set<(sample: ScrollSample) => void>>();

function phaseName(code: number): ScrollPhaseName {
  if (code === 1) {
    return "dragging";
  }
  if (code === 2) {
    return "coasting";
  }
  if (code === 3) {
    return "settling";
  }
  return "idle";
}

/** Fans one host callback out to the scroll nodes that asked for it. */
export function deliverScrollState(
  node: number,
  phase: number,
  vx: number,
  vy: number,
  ox: number,
  oy: number,
): void {
  const sample = { phase: phaseName(phase), vx, vy, ox, oy };
  for (const fn of listeners.get(node) ?? []) {
    fn(sample);
  }
}

export function onScrollState(node: number, fn: (sample: ScrollSample) => void): () => void {
  const set = listeners.get(node) ?? new Set();
  set.add(fn);
  listeners.set(node, set);
  return () => {
    set.delete(fn);
    if (set.size === 0) {
      listeners.delete(node);
    }
  };
}
