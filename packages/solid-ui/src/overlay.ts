import { host } from "@impasto/solid-renderer";

/** Moves every child. The tree does the walk; the language side does not skip nodes. */
export function moveChildren(from: number, to: number): void {
  host.reparentChildren(from, to);
}

/**
 * Puts children back on the placeholder while a layer is closing.
 * Owner disposal drops the clip before this cleanup, and that is not a failure.
 */
export function restoreChildren(from: number, to: number): void {
  if (from === 0 || to === 0) {
    return;
  }
  try {
    host.reparentChildren(from, to);
  } catch {
    // The clip or the slot was already discarded.
  }
}

/** `null` when `node` is already gone. `0` means the node is live and empty. */
export function readFirstChild(node: number): number | null {
  try {
    return host.firstChild(node);
  } catch {
    return null;
  }
}
