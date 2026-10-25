import { createEffect, createSignal } from "solid-js";
import { Text, host, type NodeHandle } from "@uifw/solid-renderer";
import { directionFor, messagesFor } from "./i18n.ts";
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
  let node = 0;

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

  createEffect(() => {
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
            color={on() ? theme().color.accent : theme().color.surface}
            radius={theme().radius.m}
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
