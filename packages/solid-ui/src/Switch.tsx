import { createTrackedEffect, createSignal } from "solid-js";
import { Text, directionFor, host, type NodeHandle } from "@impasto/solid-renderer";
import { bindFocusRing } from "./focus.ts";
import { messagesFor } from "./i18n.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A boolean control. Pass `checked` to control it, or omit it and the switch
 * keeps its own signal. `onChange` hears the next value.
 */
export function Switch(props: {
  checked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  onLabel?: string;
  offLabel?: string;
  label?: string;
}): NodeHandle {
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

  function writeSemantics(): void {
    if (node === 0) {
      return;
    }
    host.setFocusable(node, !blocked());
    host.setSemantics(
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
    <opacity alpha={blocked() ? 102 : 255}>
      <pointer
        ref={(handle) => {
          node = handle.id;
          writeSemantics();
        }}
        onClick={toggle}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKey={(key) => {
          if (key === "Enter") {
            toggle();
          }
        }}
        onText={(value) => {
          if (value === " ") {
            toggle();
          }
        }}
      >
        <padding edges={theme().space.s}>
          <decorated
            ref={(handle) => {
              box = handle.id;
            }}
            color={on() ? theme().color.accent : theme().color.surface}
            radius={theme().shape.m}
          >
            <Text
              fontSize={theme().fontSize.body}
              color={on() ? theme().color.bg : theme().color.text}
              weight={theme().weight.body}
              direction={directionFor(theme().locale)}
            >
              {phrase()}
            </Text>
          </decorated>
        </padding>
      </pointer>
    </opacity>
  );
}
