import { View as HostView } from "@impasto/solid-renderer/host";
import type { NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { ColumnProps } from "./props.ts";

/** Explicit column. The direction is a component default, so style and class can override it. */
export function Column(props: ColumnProps): NodeHandle {
  return (
    <HostView
      {...eventsOf(props)}
      ref={(node) => bindBox(node, props, { flexDirection: "column" })}
    >
      {props.children}
    </HostView>
  );
}
