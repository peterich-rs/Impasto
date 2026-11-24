import type { NodeHandle } from "@impasto/solid-renderer";
import { ExternalTexture } from "./ExternalTexture.tsx";
import type { ViewProps } from "./props.ts";
import { View } from "./View.tsx";

export interface GpuViewProps extends Omit<ViewProps, "children"> {
  /** ID registered by the host's same-device GPU producer registry. */
  producer: string;
}

/** Composites a GPU producer into a regular Impasto layout and clip. */
export function GpuView(props: GpuViewProps): NodeHandle {
  return (
    <View {...props}>
      <ExternalTexture name={`gpuview:${props.producer}`} style={{ width: "100%", height: "100%" }} />
    </View>
  );
}
