import { animate } from "../src/guard.ts";
import type { NodeHandle } from "../src/scene.ts";
import { onFrame } from "./frames.ts";
import { scene } from "./scene.ts";

/** Per-second UI spring. Not the list rubber band (`2500` / `100`). */
export const SPRING = { stiffness: 170, damping: 26 };

const pending = new Set<() => void>();

/** Runs completions queued by a spring that has no scene `animate`. */
export function flushMotion(): void {
  const batch = [...pending];
  pending.clear();
  for (const done of batch) {
    done();
  }
}

function reduced(): boolean {
  if (typeof scene.reduceMotion !== "function") {
    return false;
  }
  return scene.reduceMotion();
}

function motionEnd(id: number, prop: string, fn: () => void): number {
  const core = scene as {
    onMotionEnd?: (id: number, prop: string, fn: () => void) => number;
  };
  if (typeof core.onMotionEnd !== "function") {
    return 0;
  }
  return core.onMotionEnd(id, prop, fn);
}

function motionOff(id: number): void {
  const core = scene as { offMotionEnd?: (id: number) => void };
  if (typeof core.offMotionEnd === "function" && id !== 0) {
    core.offMotionEnd(id);
  }
}

/**
 * Springs one property. `onDone` runs when that motion ends.
 * Without `animate`, completion is the next frame. The returned function
 * unsubscribes the completion callback.
 */
export function spring(options: {
  id: NodeHandle;
  prop: string;
  from: number;
  to: number;
  stiffness?: number;
  damping?: number;
  retarget?: boolean;
  write?: (value: number) => void;
  onDone?: () => void;
}): () => void {
  const stiffness = options.stiffness ?? SPRING.stiffness;
  const damping = options.damping ?? SPRING.damping;
  let active = true;
  const finish = () => {
    if (!active) return;
    active = false;
    options.onDone?.();
  };
  const linked = typeof scene.animate === "function";
  if (reduced() || !linked) {
    options.write?.(options.to);
    if (!options.onDone) {
      scene.requestFrame();
      return () => {};
    }
    pending.add(finish);
    const stop = onFrame(() => {
      stop();
      flushMotion();
    });
    scene.requestFrame();
    return () => {
      active = false;
      pending.delete(finish);
      stop();
    };
  }
  animate(
    options.id,
    options.prop,
    options.from,
    options.to,
    stiffness,
    damping,
    options.retarget,
  );
  if (!options.onDone) return () => {};
  let token = 0;
  token = motionEnd(options.id, options.prop, () => {
    motionOff(token);
    finish();
  });
  return () => {
    if (!active) return;
    active = false;
    motionOff(token);
  };
}
