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
