import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { ViewProps } from "./props.ts";

/** Column preset comes from the native view. Author style can replace it. */
export function View(props: ViewProps): NodeHandle {
  return (
    <view {...eventsOf(props)} ref={(node) => bindBox(node, props)}>
      {props.children}
    </view>
  );
}

/** Isolates paint work for an independently updating subtree. */
export function RepaintBoundary(props: ViewProps): NodeHandle {
  return (
    <view {...eventsOf(props)} ref={(node) => {
      if (node !== null) scene.setRepaintBoundary?.(node, true);
      bindBox(node, props);
    }}>
      {props.children}
    </view>
  );
}
