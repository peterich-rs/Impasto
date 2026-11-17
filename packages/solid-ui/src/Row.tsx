import type { NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { RowProps } from "./props.ts";

/** Row preset. Direction inherits; this component does not write locale direction. */
export function Row(props: RowProps): NodeHandle {
  return (
    <view
      {...eventsOf(props)}
      ref={(node) => bindBox(node, props, { flexDirection: "row" })}
    >
      {props.children}
    </view>
  );
}
