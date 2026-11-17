import { createRenderEffect, onCleanup } from "solid-js";
import {
  scene,
  type NodeHandle,
  type NodeRef,
  type StyleInput,
  type StyleProps,
} from "@impasto/solid-renderer";

/** Sources for a target whose lifetime is shorter than its component owner. */
export function createStyleTarget(
  read: () => StyleProps | undefined,
  defaults?: () => StyleInput,
) {
  let component = 0;
  let classes = 0;
  let inline = 0;
  // Capture getters before attachment too; attachment only applies the snapshot.
  let apply = () => {};
  const update = () => {
    const spec = read();
    const value = defaults?.();
    const className = [
      spec?.class ?? "",
      ...Object.entries(spec?.classList ?? {})
        .filter(([, on]) => Boolean(on))
        .map(([name]) => name),
    ].join(" ").trim();
    const style = spec?.style;
    // Capture every nested getter without interpreting style values in JS.
    const captured = capture(style);
    const base = capture(value);
    apply = () => {
      if (component === 0) return;
      scene.setStyleSource(component, base);
      scene.setStyleSource(classes, className);
      scene.setStyleSource(inline, captured);
    };
    return apply;
  };
  createRenderEffect(update, (commit) => commit());
  function detach(): void {
    if (component !== 0) scene.removeStyleSource(component);
    if (classes !== 0) scene.removeStyleSource(classes);
    if (inline !== 0) scene.removeStyleSource(inline);
    component = classes = inline = 0;
  }
  onCleanup(detach);
  return {
    attach(node: NodeHandle) {
      detach();
      if (node === 0) return;
      component = scene.createStyleSource(node, "component");
      classes = scene.createStyleSource(node, "classes");
      inline = scene.createStyleSource(node, "inline");
      apply();
    },
    detach,
  };
}

// A structural copy preserves fragment order, undefined and null. Validation
// and resolution belong to the native style codec.
function capture<T>(value: T): T {
  if (Array.isArray(value)) return value.map(capture) as T;
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, entry]) => [key, capture(entry)]),
    ) as T;
  }
  return value;
}

/** Public ref for a panel that outlives the placeholder. */
export function createNodeRef(read: () => NodeRef | undefined) {
  let current: NodeRef | undefined;
  let node: NodeHandle | null = null;
  const clear = () => {
    const previous = current;
    current = undefined;
    if (node === null) return;
    node = null;
    previous?.(null);
  };
  const sync = (next: NodeRef | undefined) => {
    if (node === null || current === next) return;
    const previous = current;
    current = next;
    previous?.(null);
    next?.(node);
  };
  createRenderEffect(read, sync);
  onCleanup(clear);
  return {
    attach(next: NodeHandle) {
      if (node !== next) clear();
      node = next;
      sync(read());
    },
    clear,
  };
}
