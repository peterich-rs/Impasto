import { createTrackedEffect, onCleanup } from "solid-js";
import {
  readMetrics,
  scrollIntoView,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { scene } from "@impasto/solid-renderer";
import { bindBox } from "./element.tsx";
import type { KeyboardAvoidingProps } from "./props.ts";

/**
 * `padding` adds `metrics.viewInsets.bottom`.
 * `scroll` calls `scrollIntoView` on the focused node when the inset is positive.
 */
export function KeyboardAvoidingView(props: KeyboardAvoidingProps): NodeHandle {
  let bottom = 0;
  let pad = 0;
  const behavior = () => props.behavior ?? "padding";
  function apply(next: number): void {
    bottom = next;
    if (pad !== 0 && behavior() === "padding") {
      scene.patchStyle(pad, {
        paddingTop: 0,
        paddingRight: 0,
        paddingBottom: next,
        paddingLeft: 0,
      });
    }
    if (behavior() === "scroll" && next > 0) {
      const id = props.target || scene.focused();
      if (id !== 0) {
        scrollIntoView(id, 0);
      }
    }
  }
  createTrackedEffect(() => {
    behavior();
    apply(bottom);
  });
  bottom = readMetrics()?.viewInsets.bottom ?? 0;
  if (typeof scene.onWindowMetrics === "function") {
    const listener = scene.onWindowMetrics((metrics) => {
      const next = metrics.viewInsets.bottom;
      if (next !== bottom) apply(next);
    });
    onCleanup(() => scene.offWindowMetrics(listener));
  }
  return (
    <view
      ref={(node) => {
        if (node === null) return;
        pad = node;
        bindBox(node, props);
        apply(bottom);
      }}
    >
      {props.children}
    </view>
  );
}
