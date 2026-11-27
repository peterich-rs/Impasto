import type { Node } from "../src/tree.ts";
import { scene } from "./scene.ts";
import { createContext, createRoot, onCleanup, untrack, type Accessor } from "solid-js";
import { mapFor } from "@impasto/reactive";
import { bindText } from "./text.ts";
import { bindImage } from "./image.ts";
import {
  bindFrames,
  onFrame,
  onLayout as listenLayout,
  runLayouts,
} from "./frames.ts";
import { bindSlot } from "./slot.ts";
import { deliverScrollState } from "./scroll-state.ts";
import { deliverDriver } from "./driver.ts";

/** Visibility inherited by content whose host nodes move into an external overlay. */
export const SubtreeVisibleContext = createContext<Accessor<boolean>>(() => true);

const memoryPressureListeners = new Set<() => void>();
let observingMemoryPressure = false;

/** Registers cache cleanup for the host's memory pressure callback. */
export function onMemoryPressure(listener: () => void): () => void {
  if (!observingMemoryPressure) {
    scene.onMemoryPressure?.(() => {
      for (const notify of [...memoryPressureListeners]) notify();
    });
    observingMemoryPressure = true;
  }
  memoryPressureListeners.add(listener);
  return () => { memoryPressureListeners.delete(listener); };
}

const options = {
  styles: scene,
  createElement: scene.createElement,
  createTextNode: scene.createTextNode,
  replaceText: scene.setText,
  isTextNode: scene.isText,
  setProperty: scene.setProp,
  insertNode: scene.insertNode,
  removeNode: scene.remove,
  getParentNode: (node: Node) => scene.parent(node) || undefined,
  getFirstChild: (node: Node) => scene.firstChild(node) || undefined,
  getNextSibling: (node: Node) => scene.nextSibling(node) || undefined,
};
bindText(options);
bindImage(options);
export {
  insert,
  setProp,
  effect,
  memo,
  createComponent,
  createElement,
  createTextNode,
  insertNode,
} from "scene";

export function render(component: () => Node, parent: Node): () => void {
  let dispose = () => {};
  createRoot((done) => {
    dispose = done;
    scene.insert(parent, component);
    onCleanup(() => {
      let child = scene.firstChild(parent);
      while (child) {
        scene.remove(parent, child);
        child = scene.firstChild(parent);
      }
    });
  });
  return dispose;
}

export { merge as mergeProps } from "@impasto/reactive";
export { bindStyle } from "./style.ts";
export type { StyleInput, StyleProps, ViewStyle } from "../src/style.ts";
export type { AutofillHint, DisplayFeature, WindowMetrics } from "../src/scene.ts";
export function applyRef(value: unknown, element: Node): void {
  for (const ref of [value].flat(Infinity))
    if (typeof ref === "function") ref(element);
}
export function ref(read: () => unknown, element: Node): void {
  untrack(() => applyRef(read(), element));
}
export function spread(
  node: Node,
  sources: unknown,
  skipChildren?: boolean,
): void {
  const list = Array.isArray(sources) ? sources : [sources];
  const read = () =>
    Object.assign(
      {},
      ...list.map((source) =>
        typeof source === "function" ? source() : source,
      ),
    );
  let previous: Record<string, unknown> = {};
  sceneEffect(read, (next) => {
    for (const key of new Set([
      ...Object.keys(previous),
      ...Object.keys(next),
    ])) {
      if (key === "children" || next[key] === previous[key]) continue;
      if (key === "ref") applyRef(next[key], node);
      else scene.setProp(node, key, next[key]);
    }
    previous = next;
  });
  if (!skipChildren) scene.insert(node, () => read().children);
}
import {
  insert,
  effect as sceneEffect,
  createComponent,
  setProp,
  patchInlineStyle,
  patchScalarStyle,
  applyStaticClass,
  applyStaticClassList,
  instantiateTemplate,
  insert as insertRange,
} from "scene";

export { patchScalarStyle };

export function insertKeyedList<T>(parent: Node, props: Parameters<typeof mapFor<T>>[0], marker: Node): void {
  insertRange(parent, mapFor(props), marker);
}

export function bindTemplate(
  template: number | readonly unknown[],
  bind: (nodes: Node[]) => void,
): Node {
  const nodes = instantiateTemplate(template, true);
  bind(nodes);
  return nodes[0]!;
}

export function bindProp(node: Node, name: string, read: () => unknown): void {
  sceneEffect(read, (next, previous) => {
    if (next !== undefined || previous !== undefined) setProp(node, name, next, previous);
  });
}

export function bindTextValue(node: Node, read: () => string): void {
  sceneEffect(read, (next) => scene.setText(node, next));
}

export function bindInlineStyle(
  node: Node,
  selector: string | readonly string[],
  read: () => unknown,
): void {
  sceneEffect(read, (next) => patchInlineStyle(node, selector, next));
}

export function bindScalarStyle(node: Node, op: number, read: () => unknown): void {
  sceneEffect(read, next => patchScalarStyle(node, op, next));
}

export function bindStaticClass(node: Node, read: () => number): void {
  sceneEffect(read, (next) => applyStaticClass(node, next));
}

export function bindStaticClassList(
  node: Node,
  read: () => (number | undefined)[],
): void {
  sceneEffect(read, (next, previous) => {
    if (!previous || next.some((id, i) => id !== previous[i])) {
      applyStaticClassList(node, next);
    }
  });
}

export { Text, textContent, type TextGradient, type TextProps, type TextRun } from "./text.ts";
export { Image, bindImageSource, type ImageAsset, type ImageProps } from "./image.ts";

type BoxSemantics = [role?: number, label?: string, hint?: string];

export function setBoxSemantics(
  node: Node,
  role?: number,
  label?: string,
  hint?: string,
): void {
  if (role === undefined && label === undefined && hint === undefined) return;
  scene.setSemantics(node, role ?? 0, label, undefined, undefined, undefined,
    undefined, undefined, undefined, undefined, hint);
}

export function bindBoxSemantics(node: Node, read: () => BoxSemantics): void {
  let installed = false;
  sceneEffect(read, ([role, label, hint]) => {
    const labeled = role !== undefined || label !== undefined || hint !== undefined;
    if (labeled || installed) {
      scene.setSemantics(node, role ?? 0, label, undefined, undefined, undefined,
        undefined, undefined, undefined, undefined, hint);
    }
    installed = labeled;
  });
}

export { bindNodeRef } from "./ref.ts";
export type { NodeRef, UIChildren, ViewEvents } from "../src/jsx.ts";
import type { Path2DConstructor } from "../src/canvas/types.ts";

export { scene } from "./scene.ts";
export const Path2D: Path2DConstructor = scene.Path2D;
export type Path2D = InstanceType<Path2DConstructor>;
export type {
  CanvasBinding,
  CanvasComposite,
  CanvasError,
  CanvasGradient,
  CanvasPaint,
  CanvasPath,
  CanvasRenderingContext2D,
  VectorAsset,
  VectorIconDrawOptions,
  CanvasSize,
  FillRule,
  Matrix2D,
  Path2DConstructor,
  TextMetrics,
} from "../src/canvas/types.ts";
export {
  animate,
  callAsync,
  callVoid,
  flingTarget,
  layerTreeStats,
  log,
  onShortcut,
  readMetrics,
  scrollIntoView,
  setListenerErrorPolicy,
  setRecognizers,
} from "../src/guard.ts";
export { directionFor } from "../src/direction.ts";
export { onBack, type BackGesture } from "./back.ts";
export { firstFocusable, restoreFocus, takeInitialFocus } from "./focus.ts";
export { SPRING, flushMotion, spring } from "./motion.ts";
export type { Node as NodeHandle } from "../src/tree.ts";

export {
  frameListenerCount,
  layoutListenerCount,
  onFrame,
  onLayout,
} from "./frames.ts";
export type { ScrollChannel } from "../src/scene.ts";
export { listenScrollThreshold } from "./scroll-threshold.ts";
export { onScrollState } from "./scroll-state.ts";
export { listenRefresh } from "./driver.ts";
export { bindListRenderer } from "./list-binding.ts";
export type { ScrollPhaseName, ScrollSample } from "./scroll-state.ts";
export const createSlot = bindSlot({
  createElement: scene.createElement,
  insert: (parent, accessor) => {
    insert(parent, accessor);
  },
  forget: (_id) => {},
  scene,
});

/** Asks the scene for exactly one more frame after the current one. */
export function requestFrame(): void {
  scene.requestFrame();
}

let overlayId = 0;

/** Fill stack painted after the app column. Dialog moves its children here. */
export function overlayParent(): number {
  return overlayId;
}

type TopLayerFactory = () => Node;

const topLayers: TopLayerFactory[] = [];

/**
 * Mounts a layer above the app column and the overlay stack. Devtools uses
 * this so applications never change their JSX to show the debug panel. The
 * factory runs inside `boot`'s root stack.
 */
export function registerTopLayer(factory: TopLayerFactory): () => void {
  topLayers.push(factory);
  return () => {
    const index = topLayers.indexOf(factory);
    if (index >= 0) {
      topLayers.splice(index, 1);
    }
  };
}

/** Drops `node` from its parent and forgets the renderer tables for that subtree. */
export function removeNode(node: Node): void {
  const parent = scene.parent(node);
  if (parent) {
    scene.remove(parent, node);
    return;
  }
  scene.discard(node);
}

// App-facing core. Constructors return renderer nodes, not raw ids, because
// universal treats a bare number as text.
/** Registers Solid's `flush` and renders `component` into the content column. */
export function boot(component: () => Node) {
  const step = (label: string, run: () => void) => {
    try {
      run();
    } catch (error) {
      throw new Error(`${label}: ${(error as Error).stack ?? error}`);
    }
  };
  let root!: Node;
  let content!: Node;
  let overlay!: Node;
  step("root", () => {
    root = scene.createElement("stack", {
      style: {
        width: "100%",
        height: "100%",
        flex: 1,
      },
    });
  });
  scene.setComponentStyle(root, {
    gridTemplateColumns: "minmax(0, 1fr)",
    gridTemplateRows: "minmax(0, 1fr)",
  });
  step("content", () => {
    content = scene.createElement("view", { style: { gap: 0, flex: 1 } });
  });
  step("overlay", () => {
    overlay = scene.createElement("stack", {
      style: {
        width: "100%",
        height: "100%",
        flex: 1,
      },
    });
  });
  scene.setComponentStyle(overlay, {
    gridTemplateColumns: "minmax(0, 1fr)",
    gridTemplateRows: "minmax(0, 1fr)",
  });
  step("insert", () => {
    scene.insertNode(root, content);
    scene.insertNode(root, overlay);
  });
  for (const factory of topLayers) {
    scene.insert(root, factory(), 0);
  }
  overlayId = overlay;
  // The shell flushes the graph around these listeners. They only write signals.
  bindFrames((callback) => scene.onFrame(callback));
  scene.onLayout(() => {
    runLayouts();
  });
  if (typeof scene.onScrollState === "function") {
    scene.onScrollState((node, phase, vx, vy, ox, oy) => {
      deliverScrollState(node, phase, vx, vy, ox, oy);
    });
  }
  if (typeof scene.onDriver === "function") {
    scene.onDriver(
      (
        kind,
        index,
        generation,
        tag,
        i0,
        i1,
        f0,
        f1,
        f2,
        f3,
        f4,
        f5,
        s0,
        s1,
      ) => {
        deliverDriver(
          kind,
          index,
          generation,
          tag,
          i0,
          i1,
          f0,
          f1,
          f2,
          f3,
          f4,
          f5,
          s0,
          s1,
        );
      },
    );
  }
  render(component, content);
}

export const nodes = {
  column(gap: number) {
    return scene.createElement("view", { style: { gap } });
  },
  row(gap: number) {
    return scene.createElement("view", {
      style: { gap, flexDirection: "row" },
    });
  },
  stack() {
    return scene.createElement("stack");
  },
  padding(edges: number) {
    return scene.createElement("view", { style: { padding: edges } });
  },
  grid(columns: string[], rows: string[], gap: number) {
    return scene.createElement("view", {
      style: {
        display: "grid",
        gridTemplateColumns: columns.join(" "),
        gridTemplateRows: rows.join(" "),
        gap,
      },
    });
  },
  pointerRegion() {
    return scene.createElement("view");
  },
  onFrame(fn: (targetMs: number, intervalMs: number) => void) {
    onFrame(fn);
  },
  onLayout(fn: () => void) {
    listenLayout(fn);
  },
};

// builtIns import these names from moduleName. rc.17 calls the boundary Errored.
export {
  Errored,
  Errored as ErrorBoundary,
  For,
  Match,
  Show,
  Switch,
  onCleanup,
} from "@impasto/reactive";

export function Dynamic(props: {
  component?: ((props: Record<string, unknown>) => Node) | null;
}): Node {
  const component = props.component;
  if (typeof component !== "function") {
    return scene.createTextNode("");
  }
  const { component: _component, ...rest } = props;
  return createComponent(component, rest as never);
}
