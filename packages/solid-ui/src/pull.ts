export type PullEdge = "start" | "end";

export type PullDecision = "ignore" | "hold" | "refresh";

const DEAD_ZONE = 0.5;

export const PULL_STIFFNESS = 2500;
export const PULL_DAMPING = 100;

/** `start` opens when overscroll is negative. `end` opens when it is positive. */
export function pullRelease(input: {
  edge: PullEdge;
  overscroll: number;
  threshold: number;
  busy: boolean;
}): PullDecision {
  const opening = input.edge === "start" ? input.overscroll < -DEAD_ZONE : input.overscroll > DEAD_ZONE;
  if (!opening) {
    return "ignore";
  }
  const distance = input.edge === "start" ? -input.overscroll : input.overscroll;
  if (input.busy) {
    // A short band during an in-flight refresh must spring shut. Holding every
    // open gap traps the list at the edge until the callback returns.
    return distance >= input.threshold ? "hold" : "ignore";
  }
  if (distance >= input.threshold) {
    return "refresh";
  }
  return "ignore";
}

/** Pixels the gap is open. Zero inside the dead zone, so a resting scroll stays shut. */
export function pullGap(edge: PullEdge, overscroll: number): number {
  const distance = edge === "start" ? -overscroll : overscroll;
  return distance > DEAD_ZONE ? distance : 0;
}

const PROGRESS_STEPS = 8;

/** Coarse progress. Each step is one text repaint, not one pointer sample. */
export function quantizeProgress(value: number): number {
  if (!(value > 0)) {
    return 0;
  }
  const clamped = Math.min(1, value);
  return Math.round(clamped * PROGRESS_STEPS) / PROGRESS_STEPS;
}

export function pullProgress(edge: PullEdge, overscroll: number, threshold: number): number {
  if (!(threshold > 0)) {
    return 0;
  }
  const distance = edge === "start" ? Math.max(0, -overscroll) : Math.max(0, overscroll);
  return Math.min(1, distance / threshold);
}

export function gapOpen(edge: PullEdge, overscroll: number): boolean {
  return edge === "start" ? overscroll < -DEAD_ZONE : overscroll > DEAD_ZONE;
}
