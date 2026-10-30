import { createTrackedEffect } from "solid-js";
import { directionFor, type NodeHandle } from "@impasto/solid-renderer";
import { coreHost } from "./host-ext.ts";
import { useTheme } from "./use-theme.ts";

type Justify = "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly";

/**
 * A row that mirrors children in RTL locales via `setReverse`.
 * Does not scale or flip through a transform.
 */
export function Row(props: {
  gap?: number;
  fill?: boolean;
  justify?: Justify;
  children?: unknown;
}): NodeHandle {
  const theme = useTheme();
  let node = 0;
  createTrackedEffect(() => {
    if (node !== 0) {
      coreHost.setReverse(node, directionFor(theme().locale) === "rtl");
    }
  });
  return (
    <row
      gap={props.gap}
      fill={props.fill}
      justify={props.justify}
      ref={(handle) => {
        node = handle.id;
        coreHost.setReverse(handle.id, directionFor(theme().locale) === "rtl");
      }}
    >
      {props.children}
    </row>
  );
}
