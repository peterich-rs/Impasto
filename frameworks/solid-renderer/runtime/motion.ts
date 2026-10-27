import { animate, type NodeHandle } from "../src/guard.ts";
import { onFrame } from "./frames.ts";
import { host } from "./host.ts";

/** Same pair as `core::motion::OVERSCROLL_STIFFNESS` and `OVERSCROLL_DAMPING`. */
export const SPRING = { stiffness: 170, damping: 26 };

const pending: Array<() => void> = [];

/** Runs completions queued by a spring that has no host `animate`. */
export function flushMotion(): void {
  const batch = pending.splice(0);
  for (const done of batch) {
    done();
  }
}

function reduced(): boolean {
  if (typeof host.reduceMotion !== "function") {
    return false;
  }
  return host.reduceMotion();
}

function motionEnd(id: number, prop: string, fn: () => void): number {
  const core = host as { onMotionEnd?: (id: number, prop: string, fn: () => void) => number };
  if (typeof core.onMotionEnd !== "function") {
    return 0;
  }
  return core.onMotionEnd(id, prop, fn);
}

function motionOff(id: number): void {
  const core = host as { offMotionEnd?: (id: number) => void };
  if (typeof core.offMotionEnd === "function" && id !== 0) {
    core.offMotionEnd(id);
  }
}

/**
 * Springs one property. `onDone` runs when that motion ends.
 * Without `animate`, completion is the next frame.
 */
export function spring(options: {
  id: NodeHandle;
  prop: string;
  from: number;
  to: number;
  stiffness?: number;
  damping?: number;
  write?: (value: number) => void;
  onDone?: () => void;
}): void {
  const stiffness = options.stiffness ?? SPRING.stiffness;
  const damping = options.damping ?? SPRING.damping;
  const finish = () => options.onDone?.();
  const linked = typeof host.animate === "function";
  if (reduced() || !linked) {
    options.write?.(options.to);
    if (options.onDone) {
      pending.push(options.onDone);
    }
    const stop = onFrame(() => {
      stop();
      flushMotion();
    });
    host.requestFrame();
    return;
  }
  animate(options.id, options.prop, options.from, options.to, stiffness, damping);
  let token = 0;
  token = motionEnd(options.id, options.prop, () => {
    motionOff(token);
    finish();
  });
}
