import { onCleanup } from "solid-js";
import {
  createElement,
  scene,
  spread,
  type CanvasBinding,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { CanvasProps } from "./props.ts";

/** A view whose content box is recorded by `draw` after layout. */
export function Canvas(props: CanvasProps): NodeHandle {
  const node = createElement("view");
  let binding: CanvasBinding | null = null;
  onCleanup(() => {
    binding?.release();
    binding = null;
    props.controller?.(null);
  });
  const attach = (element: NodeHandle | null) => {
    bindBox(element, props);
    if (element === null) return;
    binding = scene.bindCanvas(
      element,
      (context, size) => props.draw(context, size),
      (error) => {
        const handler = props.onDrawError;
        if (handler == null) return false;
        handler(error);
      },
    );
    props.controller?.(binding);
  };
  spread(node, () => ({ ...eventsOf(props), ref: attach }), true);
  return node;
}
