import { createTrackedEffect, createSignal } from "solid-js";
import { directionFor, type NodeHandle } from "@uifw/solid-renderer";
import { coreHost } from "./host-ext.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A transform whose horizontal scale mirrors in RTL when `mirror` is set.
 */
export function Icon(props: { mirror?: boolean; children?: unknown }): NodeHandle {
  const theme = useTheme();
  const [iconId, setIconId] = createSignal(0);
  createTrackedEffect(() => {
    const icon = iconId();
    const locale = theme().locale;
    const mirror = !!props.mirror;
    if (icon !== 0) {
      coreHost.setScaleXY(icon, directionFor(locale) === "rtl" && mirror ? -1 : 1, 1);
    }
  });
  return (
    <transform
      ref={(handle) => {
        setIconId(handle.id);
      }}
    >
      {props.children}
    </transform>
  );
}
