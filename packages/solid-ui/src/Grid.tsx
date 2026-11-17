import type { NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { GridProps } from "./props.ts";

/** Grid preset. Tracks and placement belong on `style`. */
export function Grid(props: GridProps): NodeHandle {
  return (
    <view
      {...eventsOf(props)}
      ref={(node) => bindBox(node, props, { display: "grid" })}
    >
      {props.children}
    </view>
  );
}
