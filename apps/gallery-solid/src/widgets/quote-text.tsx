import { onCleanup, createTrackedEffect } from "solid-js";
import { Text, type NodeHandle } from "@impasto/solid-renderer";
import type { QuoteBinder } from "../state/binder.ts";

/**
 * A quote-driven text cell. The binder owns the text and the color after
 * mount, so this component never sets `color`. On recycle the render effect
 * re-runs: old symbol unbinds, new symbol binds and writes immediately
 * (write-on-bind keeps pooled rows from showing stale prices).
 */
export function QuoteText(props: {
  symbol: () => string;
  kind: "last" | "pct" | "chg";
  fontSize: number;
  weight?: number;
  align?: "left" | "right";
  binder: () => QuoteBinder;
}): NodeHandle {
  const node = Text({ fontSize: props.fontSize, children: "—" });
  createTrackedEffect(() => {
    const stop = props.binder().bind(props.symbol(), {
      [props.kind]: node.id,
      flashNode: node.id,
    });
    onCleanup(stop);
  });
  return node;
}
