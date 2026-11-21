import { scene, type NodeHandle, type StyleProps } from "@impasto/solid-renderer";
import { createTrackedEffect } from "solid-js";
import { bindBox } from "./element.tsx";

export interface PlatformViewProps extends StyleProps {
  kind: string;
}

/** Reserves a layout box for a native view registered by the platform host. */
export function PlatformView(props: PlatformViewProps): NodeHandle {
  return (
    <view
      ref={(node) => {
        if (node == null) return;
        bindBox(node, props);
        createTrackedEffect(() => scene.setPlatformView(node, props.kind));
      }}
    />
  );
}
