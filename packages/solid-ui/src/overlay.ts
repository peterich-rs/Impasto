import { host } from "@impasto/solid-renderer";

/** Moves every child. The tree does the walk; the language side does not skip nodes. */
export function moveChildren(from: number, to: number): void {
  host.reparentChildren(from, to);
}
