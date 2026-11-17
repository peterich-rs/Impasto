import { scene } from "./scene.ts";
import { createRenderEffect, onCleanup } from "solid-js";
import type { Node } from "../src/tree.ts";
import type { StyleInput, StyleProps } from "../src/style.ts";

/** One binding owns its sources; cleanup leaves other writers intact. */
export function bindStyle(
  node: Node,
  spec: StyleProps & { defaults?: StyleInput },
): void {
  bindStyleTo(scene, node, spec);
}

export function bindStyleTo(
  port: Pick<
    typeof scene,
    "createStyleSource" | "setStyleSource" | "removeStyleSource"
  >,
  node: Node,
  spec: StyleProps & { defaults?: StyleInput },
): void {
  const defaults = port.createStyleSource(node, "component");
  const classes = port.createStyleSource(node, "classes");
  const inline = port.createStyleSource(node, "inline");
  onCleanup(() => {
    port.removeStyleSource(defaults);
    port.removeStyleSource(classes);
    port.removeStyleSource(inline);
  });
  createRenderEffect(
    () => ({
      defaults: spec.defaults,
      classes: [
        spec.class ?? "",
        ...Object.entries(spec.classList ?? {})
          .filter(([, on]) => on)
          .map(([name]) => name),
      ]
        .join(" ")
        .trim(),
      style: spec.style,
    }),
    (value) => {
      port.setStyleSource(defaults, value.defaults);
      port.setStyleSource(classes, value.classes);
      port.setStyleSource(inline, value.style);
    },
  );
}
