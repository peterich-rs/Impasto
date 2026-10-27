import { createTrackedEffect, onCleanup } from "solid-js";
import { onFrame, readMetrics, scrollIntoView, type NodeHandle } from "@uifw/solid-renderer";
import { host } from "@uifw/solid-renderer";

/**
 * `padding` adds `metrics.viewInsets.bottom`.
 * `scroll` calls `scrollIntoView` on the focused node when the inset is positive.
 */
export function KeyboardAvoidingView(props: {
  behavior?: "padding" | "scroll";
  target?: number;
  children?: NodeHandle;
}): NodeHandle {
  let bottom = 0;
  let pad = 0;
  const behavior = () => props.behavior ?? "padding";

  function apply(next: number): void {
    bottom = next;
    if (pad !== 0 && behavior() === "padding") {
      host.setPaddingEdges(pad, 0, 0, next, 0);
    }
    if (behavior() === "scroll" && next > 0) {
      const id = props.target || host.focused();
      if (id !== 0) {
        scrollIntoView(id, 0);
      }
    }
  }

  createTrackedEffect(() => {
    behavior();
    apply(bottom);
  });

  onCleanup(
    onFrame(() => {
      const metrics = readMetrics();
      const next = metrics?.viewInsets.bottom ?? 0;
      if (next !== bottom) {
        apply(next);
      }
    }),
  );

  return (
    <padding
      ref={(node) => {
        pad = node.id;
        apply(bottom);
      }}
    >
      {props.children}
    </padding>
  );
}
