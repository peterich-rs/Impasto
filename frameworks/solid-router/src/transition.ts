/** Exit alpha in the stack transition is 0.6, written to core as 0–255. */
export const STACK_EXIT_ALPHA = Math.round(0.6 * 255);

/** Page travel when layout has not measured the mover yet. */
export function travelDistance(measured: number, viewport: number): number {
  if (measured >= 1) {
    return measured;
  }
  if (viewport >= 1) {
    return viewport;
  }
  return 360;
}

/** Horizontal travel for a stack push or pop. `distance` is the page width. */
export function stackMotion(
  phase: "enter" | "exit" | "cover",
  distance = 100,
): Array<{ prop: string; from: number; to: number }> {
  const travel = Number.isFinite(distance) && distance > 0 ? distance : 100;
  if (phase === "enter") {
    return [{ prop: "translateX", from: travel, to: 0 }];
  }
  return [
    { prop: "translateX", from: 0, to: -Math.round(travel * 0.3) },
    { prop: "opacity", from: 255, to: STACK_EXIT_ALPHA },
  ];
}
