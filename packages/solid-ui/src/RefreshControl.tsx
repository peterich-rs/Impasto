import { createRenderEffect, onCleanup } from "solid-js";
import type { NodeHandle } from "@impasto/solid-renderer";
import { refreshListener, type ScrollController } from "./ScrollController.ts";

const DEFAULT_THRESHOLD = 64;

/**
 * Pull distance is the controller overscroll. The armed threshold and the
 * idle commit live on the controller.
 */
export function RefreshControl(props: {
  controller: ScrollController;
  threshold?: number;
  onRefresh?: () => void;
  children?: unknown;
}): NodeHandle {
  createRenderEffect(
    () => props.threshold ?? DEFAULT_THRESHOLD,
    (threshold) => {
      props.controller.setPull(1, threshold);
    },
  );
  onCleanup(refreshListener(props.controller, () => props.onRefresh?.()));
  const pull = () => Math.max(0, -props.controller.overscroll().y);

  return (
    <column>
      <sized w={1} h={pull()} />
      {props.children}
    </column>
  ) as unknown as NodeHandle;
}
