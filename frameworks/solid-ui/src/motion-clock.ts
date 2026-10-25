export type MotionHooks = {
  reduceMotion: () => boolean;
  onFrame: (fn: (targetMs: number, intervalMs: number) => void) => () => void;
  requestFrame: () => void;
};

let bound: MotionHooks | null = null;
let testHooks: MotionHooks | null = null;

/** Tests replace the clock. Pass null to fall back to `bindMotion`. */
export function installMotionTestHooks(next: MotionHooks | null): void {
  testHooks = next;
}

export function bindMotion(next: MotionHooks): void {
  bound = next;
}

function clock(): MotionHooks | null {
  return testHooks ?? bound;
}

/**
 * Writes `t` from 0 to 1 across `durationMs`.
 * `reduceMotion` or a non-positive duration writes 1 once and does not schedule a frame.
 */
export function transition(write: (t: number) => void, durationMs: number): () => void {
  const active = clock();
  if (active === null) {
    throw new Error("motion is not bound");
  }
  if (active.reduceMotion() || !(durationMs > 0)) {
    write(1);
    return () => {};
  }
  let start: number | null = null;
  let stop = () => {};
  stop = active.onFrame((targetMs) => {
    if (start === null) {
      start = targetMs;
    }
    const t = Math.min(1, Math.max(0, (targetMs - start) / durationMs));
    write(t);
    if (t >= 1) {
      stop();
    }
  });
  active.requestFrame();
  return () => {
    stop();
  };
}
