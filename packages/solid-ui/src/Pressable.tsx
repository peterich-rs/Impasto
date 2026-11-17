import { createSignal } from "solid-js";
import { type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import { bindPressable } from "./pressable.ts";
import type { PressableProps } from "./props.ts";

/** One view. Keyboard and disabled behavior run before the author's callbacks. */
export function Pressable(props: PressableProps): NodeHandle {
  const [pressed, setPressed] = createSignal(false);
  const [focused, setFocused] = createSignal(false);
  let node = 0;
  const blocked = () => !!props.disabled;
  const press = bindPressable({
    node: () => node,
    disabled: blocked,
    focused,
    setPressed,
    setFocused,
    onActivate: () => props.onClick?.(),
    events: props,
  });
  return (
    <view
      {...eventsOf(props)}
      focusable={!blocked()}
      ref={(handle) => {
        if (handle === null) return;
        node = handle;
        bindBox(handle, props, undefined, blocked);
      }}
      onPress={press.onPress}
      onRelease={press.onRelease}
      onClick={press.onClick}
      onFocus={press.onFocus}
      onBlur={press.onBlur}
      onKey={press.onKey}
      onText={press.onText}
    >
      {props.children}
    </view>
  );
}
