import type { NodeHandle } from "@impasto/solid-renderer";
import { FieldShell } from "./field/shell.tsx";
import type { TextAreaProps } from "./props.ts";

/**
 * Multiline field. Enter inserts a newline when the action is `newline`.
 * The outer control is a scroll. A vertical drag is left for a parent list;
 * a long press asks the scene for text selection.
 */
export function TextArea(props: TextAreaProps): NodeHandle {
  return (
    <FieldShell
      {...props}
      multiline
      action={props.action ?? "newline"}
      rows={props.rows ?? 3}
    />
  );
}
