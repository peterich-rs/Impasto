import { createSignal } from "solid-js";
import { Text, type NodeHandle } from "@uifw/solid-renderer";
import { BUTTON_FILL, BUTTON_LABEL, FONT_SIZE } from "@uifw/solid-renderer/defaults";

/**
 * A boolean control. Pass `checked` to control it, or omit it and the switch
 * keeps its own signal. `onChange` hears the next value.
 */
export function Switch(props: {
  checked?: boolean;
  onChange?: (checked: boolean) => void;
}): NodeHandle {
  const [local, setLocal] = createSignal(false);
  const on = () => (props.checked === undefined ? local() : props.checked);
  return (
    <pointer
      onClick={() => {
        const next = !on();
        if (props.checked === undefined) {
          setLocal(next);
        }
        props.onChange?.(next);
      }}
    >
      <padding edges={8}>
        <decorated color={on() ? BUTTON_LABEL : BUTTON_FILL} radius={6}>
          <Text fontSize={FONT_SIZE} color={on() ? BUTTON_FILL : BUTTON_LABEL}>
            {on() ? "On" : "Off"}
          </Text>
        </decorated>
      </padding>
    </pointer>
  );
}
