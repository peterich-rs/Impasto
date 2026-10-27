import type { NodeHandle } from "@uifw/solid-renderer";
import type { FieldProps } from "./field/shell.tsx";
import { FieldShell } from "./field/shell.tsx";

/**
 * Multiline field. Enter inserts a newline when the action is `newline`.
 * The outer control is a scroll. A vertical drag is left for a parent list;
 * a long press asks the host for text selection.
 */
export function TextArea(props: Omit<FieldProps, "multiline">): NodeHandle {
  return (
    <FieldShell
      {...props}
      multiline
      action={props.action ?? "newline"}
      rows={props.rows ?? 3}
    />
  );
}
