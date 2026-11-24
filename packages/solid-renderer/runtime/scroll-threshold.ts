import { scene } from "./scene.ts";

const listeners = new Map<number, (active: boolean) => void>();
let installed = false;

/** Registers a discrete observation of the native scroll position. */
export function listenScrollThreshold(
  node: number,
  source: "scrollX" | "scrollY" | "overscrollX" | "overscrollY",
  value: number,
  fromEnd: boolean,
  hysteresis: number,
  listener: (active: boolean) => void,
): () => void {
  if (!installed) {
    scene.onScrollThreshold((id, active) => listeners.get(id)?.(active));
    installed = true;
  }
  const id = scene.addScrollThreshold(node, source, value, fromEnd, hysteresis);
  listeners.set(id, listener);
  return () => {
    if (listeners.delete(id)) scene.removeScrollThreshold(id);
  };
}
