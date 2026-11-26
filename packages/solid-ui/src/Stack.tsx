import { Stack as HostStack } from "@impasto/solid-renderer/host";
import type { NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { StackProps } from "./props.ts";

/** One-cell stack. Track templates stay on the native preset. */
export function Stack(props: StackProps): NodeHandle {
  return (
    <HostStack {...eventsOf(props)} ref={(node) => bindBox(node, props)}>
      {props.children}
    </HostStack>
  );
}
