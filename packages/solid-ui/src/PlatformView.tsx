import { View as HostView } from "@impasto/solid-renderer/host";
import { scene, type NodeHandle, type StyleProps } from "@impasto/solid-renderer";
import { createTrackedEffect } from "solid-js";
import { bindBox } from "./element.tsx";

export interface PlatformViewProps extends StyleProps {
  kind: string;
  /** SVG path data in the platform view's local logical points. */
  clipPath?: string;
}

/** Reserves a layout box for a native view registered by the platform host. */
export function PlatformView(props: PlatformViewProps): NodeHandle {
  return (
    <HostView
      ref={(node) => {
        if (node == null) return;
        bindBox(node, props);
        createTrackedEffect(() => scene.setPlatformView(node, props.kind, props.clipPath));
      }}
    />
  );
}
