import { createTrackedEffect, createSignal } from "solid-js";
import {
  Text,
  bindStyle,
  scene,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { bindBox } from "./element.tsx";
import { bindFocusRing } from "./focus.ts";
import { bindPressable } from "./pressable.ts";
import { messagesFor } from "./i18n.ts";
import type { SwitchProps } from "./props.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A boolean control. Pass `checked` to control it, or omit it and the switch
 * keeps its own signal. `onChange` hears the next value.
 */
export function Switch(props: SwitchProps): NodeHandle {
  const theme = useTheme();
  const [local, setLocal] = createSignal(false);
  const on = () => (props.checked === undefined ? local() : props.checked);
  const msgs = () => messagesFor(theme().locale);
  const onLabel = () => props.onLabel ?? msgs().switchOn;
  const offLabel = () => props.offLabel ?? msgs().switchOff;
  const phrase = () => (on() ? onLabel() : offLabel());
  const label = () => {
    const value = props.label ?? phrase();
    return value.length > 0 ? value : phrase();
  };
  const blocked = () => !!props.disabled;
  const [focused, setFocused] = createSignal(false);
  let node = 0;
  let box = 0;
  bindFocusRing(() => box, focused);
  function toggle(): void {
    if (blocked()) {
      return;
    }
    const next = !on();
    if (props.checked === undefined) {
      setLocal(next);
    }
    props.onChange?.(next);
  }
  const press = bindPressable({
    events: {},
    node: () => node,
    disabled: blocked,
    focused,
    setPressed: () => {},
    setFocused,
    onActivate: toggle,
  });
  function writeSemantics(): void {
    if (node === 0) {
      return;
    }
    scene.setFocusable(node, !blocked());
    scene.setSemantics(
      node,
      Role.Switch,
      label(),
      phrase(),
      on() ? 1 : 0,
      undefined,
      undefined,
      blocked() ? 1 : 0,
    );
  }
  createTrackedEffect(() => {
    writeSemantics();
  });
  return (
    <view
      ref={(handle) => {
        if (handle === null) return;
        node = handle;
        bindBox(handle, props, () => ({
          padding: theme().space.s,
          opacity: blocked() ? 0.4 : 1,
          color: on() ? theme().color.bg : theme().color.text,
          fontSize: theme().fontSize.body,
          fontWeight: theme().weight.body,
        }));
        writeSemantics();
      }}
      onClick={press.onClick}
      onFocus={press.onFocus}
      onBlur={press.onBlur}
      onKey={press.onKey}
      onText={press.onText}
    >
      <view
        ref={(handle) => {
          if (handle === null) return;
          box = handle;
          bindStyle(handle, {
            get defaults() {
              return {
                backgroundColor: on()
                  ? theme().color.accent
                  : theme().color.surface,
                borderRadius: theme().shape.m,
              };
            },
            get class() {
              return props.trackProps?.class;
            },
            get classList() {
              return props.trackProps?.classList;
            },
            get style() {
              return props.trackProps?.style;
            },
          });
        }}
      >
        <Text
          class={props.labelProps?.class}
          classList={props.labelProps?.classList}
          style={props.labelProps?.style}
        >
          {phrase()}
        </Text>
      </view>
    </view>
  );
}
