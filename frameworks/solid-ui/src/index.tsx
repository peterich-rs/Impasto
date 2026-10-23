import { createSignal } from "solid-js";
import { Text, type NodeHandle } from "@uifw/solid-renderer";
import { BUTTON_FILL, BUTTON_LABEL, BUTTON_PRESSED, FONT_SIZE } from "@uifw/solid-renderer/defaults";

export { Field } from "./Field.tsx";
export { ScrollArea } from "./ScrollArea.tsx";

export function Button(props: { label: string; onClick: () => void }): NodeHandle {
  const [pressed, setPressed] = createSignal(false);
  return (
    <pointer
      onPress={() => setPressed(true)}
      onRelease={() => setPressed(false)}
      onClick={props.onClick}
    >
      <padding edges={8}>
        <decorated color={pressed() ? BUTTON_PRESSED : BUTTON_FILL} radius={6}>
          <Text fontSize={FONT_SIZE} color={BUTTON_LABEL}>
            {props.label}
          </Text>
        </decorated>
      </padding>
    </pointer>
  );
}
