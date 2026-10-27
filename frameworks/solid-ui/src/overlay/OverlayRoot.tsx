import { type JSX, createContext, useContext } from "solid-js";
import type { NodeHandle } from "@uifw/solid-renderer";
import { closeTopModal, overlayEntries, registerOverlay, type OverlayEntry } from "./registry.ts";

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
export function OverlayRoot(props: { children?: NodeHandle }): NodeHandle {
  return (
    <OverlayContext value={api}>
      <stack fill>{props.children}</stack>
    </OverlayContext>
  );
}

export function useOverlay(): OverlayApi {
  return useContext(OverlayContext);
}

export type { JSX };
