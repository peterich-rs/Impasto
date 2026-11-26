import { Stack as HostStack } from "@impasto/solid-renderer/host";
import { createContext, useContext } from "solid-js";
import type { NodeHandle } from "@impasto/solid-renderer";
import { bindBox } from "../element.tsx";
import type { OverlayRootProps } from "../props.ts";
import {
  closeTopModal,
  overlayEntries,
  registerOverlay,
  type OverlayEntry,
} from "./registry.ts";
export type OverlayApi = {
  register: typeof registerOverlay;
  closeTopModal: typeof closeTopModal;
  entries: () => readonly OverlayEntry[];
};
const api: OverlayApi = {
  register: registerOverlay,
  closeTopModal,
  entries: overlayEntries,
};
const OverlayContext = createContext<OverlayApi>(api);

/** Hosts overlay state for Dialog, Sheet, Menu, Toast, and Tooltip. */
export function OverlayRoot(props: OverlayRootProps): NodeHandle {
  return (
    <OverlayContext value={api}>
      <HostStack
        ref={(node) =>
          bindBox(node, props, {
            width: "100%",
            height: "100%",
            flex: 1,
            flexShrink: 1,
            minWidth: 0,
            minHeight: 0,
            gridTemplateColumns: "minmax(0, 1fr)",
            gridTemplateRows: "minmax(0, 1fr)",
          })
        }
      >
        {props.children}
      </HostStack>
    </OverlayContext>
  );
}
export function useOverlay(): OverlayApi {
  return useContext(OverlayContext);
}
export type { JSX } from "@impasto/solid-renderer/jsx";
