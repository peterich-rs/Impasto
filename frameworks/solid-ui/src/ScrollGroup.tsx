import type { ScrollController } from "./ScrollController.ts";

/**
 * Drag remainder after the inner scroll already holds the lock.
 * Lock-time edge yield stays in `Tree::scroll_for`.
 * Returns whether the parent consumed the overflow. Otherwise the child rubber-bands.
 */
export function consumeScrollOverflow(
  parent: ScrollController,
  overflowX: number,
  overflowY: number,
): boolean {
  if (parent.node() === 0) {
    return false;
  }
  return parent.consume(overflowX, overflowY);
}
