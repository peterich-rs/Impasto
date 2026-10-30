const ticks = new Map<number, (targetMs: number, intervalMs: number) => void>();
let nextTick = 1;

/** Runs after Solid flushes, once per frame the host delivers. */
export function onFrame(fn: (targetMs: number, intervalMs: number) => void): () => void {
  const id = nextTick;
  nextTick += 1;
  ticks.set(id, fn);
  return () => {
    ticks.delete(id);
  };
}

/** Listeners still registered. Tests use this after mount and unmount. */
export function frameListenerCount(): number {
  return ticks.size;
}

/**
 * Calls each listener. A listener that throws is removed and the error propagates.
 */
export function runFrames(targetMs: number, intervalMs: number): void {
  for (const [id, fn] of ticks) {
    try {
      fn(targetMs, intervalMs);
    } catch (error) {
      ticks.delete(id);
      throw error;
    }
  }
}

const layouts = new Map<number, () => void>();

/** Runs after layout, once on a frame that actually laid out. */
export function onLayout(fn: () => void): () => void {
  const id = nextTick;
  nextTick += 1;
  layouts.set(id, fn);
  return () => {
    layouts.delete(id);
  };
}

export function layoutListenerCount(): number {
  return layouts.size;
}

/** A listener that throws is removed and the error propagates. */
export function runLayouts(): void {
  for (const [id, fn] of layouts) {
    try {
      fn();
    } catch (error) {
      layouts.delete(id);
      throw error;
    }
  }
}
