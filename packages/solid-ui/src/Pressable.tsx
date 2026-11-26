import { View as HostView } from "@impasto/solid-renderer/host";
import { createSignal } from "solid-js";
import { type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, bindViewEvents } from "./element.tsx";
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
    <HostView
      ref={(handle) => {
        if (handle === null) return;
        node = handle;
        bindViewEvents(handle, props, press);
        bindBox(handle, props, undefined, blocked);
      }}
    >
      {props.children}
    </HostView>
  );
}
