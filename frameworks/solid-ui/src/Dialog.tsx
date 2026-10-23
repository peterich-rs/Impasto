import { Show } from "solid-js";
import type { NodeHandle } from "@uifw/solid-renderer";

/**
 * A full-size scrim over a column of content. Closing writes the signal the
 * caller passed in; this component does not trap focus.
 */
export function Dialog(props: {
  open: boolean;
  onClose: () => void;
  children?: NodeHandle;
}): NodeHandle {
  return (
    <stack fill>
      <Show when={props.open}>
        <stack fill>
          <pointer fill onClick={() => props.onClose()}>
            <decorated color="#00000080" radius={0} fill />
          </pointer>
          <column gap={8}>{props.children}</column>
        </stack>
      </Show>
    </stack>
  );
}
