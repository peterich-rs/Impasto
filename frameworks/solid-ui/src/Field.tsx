import type { FieldProps } from "./field/shell.tsx";
import { FieldShell } from "./field/shell.tsx";
import type { NodeHandle } from "@uifw/solid-renderer";

export type { FieldProps };

/**
 * A text field. The string, selection, and composing range live in `core`'s
 * text editor. Masks stay raw during composition and commit with opcode 38.
 */
export function Field(props: FieldProps): NodeHandle {
  return <FieldShell {...props} />;
}
