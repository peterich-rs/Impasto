import type { CoreHost, NodeHandle } from "./host.ts";

export type AnimPropName =
  | "scrollX"
  | "scrollY"
  | "opacity"
  | "translateX"
  | "translateY"
  | "rotate"
  | "scale"
  | "overscrollX"
  | "overscrollY";

export type LayerTreeStats = { epoch: number; layers: number; pictures: number };

export type RecognizerSpec = {
  tap?: boolean;
  longPress?: boolean;
  drag?: "none" | "free" | "horizontal" | "vertical";
  /** Double-tap window in milliseconds. Omitted means the host default. */
  doubleTapMs?: number;
};

export type SemanticsHint = {
  order?: number;
  live?: "off" | "polite" | "assertive";
};

export type ViewMetrics = {
  width: number;
  height: number;
  viewInsets: { top: number; right: number; bottom: number; left: number };
};

type CoreBag = Partial<CoreHost> & Record<string, unknown>;

function bag(): CoreBag {
  const core = (globalThis as { core?: CoreBag }).core;
  return core ?? {};
}

function fn(name: string): ((...args: unknown[]) => unknown) | null {
  const value = bag()[name];
  return typeof value === "function" ? (value as (...args: unknown[]) => unknown) : null;
}

/** Missing host functions no-op. Present functions still throw their own errors. */
export function callVoid(name: string, args: unknown[]): void {
  const impl = fn(name);
  if (impl === null) {
    return;
  }
  impl.apply(bag(), args);
}

/** Missing async host functions reject. They do not throw synchronously. */
export function callAsync(name: string, args: unknown[]): Promise<unknown> {
  const impl = fn(name);
  if (impl === null) {
    return Promise.reject(new Error(`${name} is not linked`));
  }
  try {
    return Promise.resolve(impl.apply(bag(), args));
  } catch (error) {
    return Promise.reject(error);
  }
}

export function scrollIntoView(id: NodeHandle, edge: number): void {
  callVoid("scrollIntoView", [id, edge]);
}

export function animate(
  id: NodeHandle,
  prop: AnimPropName | string,
  from: number,
  to: number,
  stiffness: number,
  damping: number,
): void {
  callVoid("animate", [id, prop, from, to, stiffness, damping]);
}

export function flingTarget(id: NodeHandle): { x: number; y: number } | null {
  const impl = fn("flingTarget");
  if (impl === null) {
    return null;
  }
  const value = impl.apply(bag(), [id]);
  if (value === null || value === undefined) {
    return null;
  }
  const point = value as { x?: unknown; y?: unknown };
  if (typeof point.x !== "number" || typeof point.y !== "number") {
    return null;
  }
  return { x: point.x, y: point.y };
}

export function setListenerErrorPolicy(policy: "report" | "freeze"): void {
  callVoid("setListenerErrorPolicy", [policy]);
}

export function layerTreeStats(): LayerTreeStats {
  const impl = fn("layerTreeStats");
  if (impl === null) {
    return { epoch: 0, layers: 0, pictures: 0 };
  }
  const value = impl.apply(bag(), []) as Partial<LayerTreeStats> | null;
  return {
    epoch: typeof value?.epoch === "number" ? value.epoch : 0,
    layers: typeof value?.layers === "number" ? value.layers : 0,
    pictures: typeof value?.pictures === "number" ? value.pictures : 0,
  };
}

export function onShortcut(spec: string, cb: () => void): void {
  callVoid("onShortcut", [spec, cb]);
}

export function setRecognizers(node: NodeHandle, spec: RecognizerSpec): void {
  callVoid("setRecognizers", [node, spec]);
}

export function setStroke(node: NodeHandle, width: number, color: number[]): void {
  callVoid("setStroke", [node, width, color]);
}

export function log(level: string, message: string): void {
  callVoid("log", [level, message]);
}

export function readMetrics(): ViewMetrics | null {
  const impl = fn("metrics");
  if (impl === null) {
    return null;
  }
  const value = impl.apply(bag(), []) as Partial<ViewMetrics> | null;
  const insets = value?.viewInsets;
  if (!insets) {
    return null;
  }
  return {
    width: typeof value.width === "number" ? value.width : 0,
    height: typeof value.height === "number" ? value.height : 0,
    viewInsets: {
      top: insets.top ?? 0,
      right: insets.right ?? 0,
      bottom: insets.bottom ?? 0,
      left: insets.left ?? 0,
    },
  };
}
