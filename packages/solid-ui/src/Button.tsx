import { View as HostView } from "@impasto/solid-renderer/host";
import { createTrackedEffect, createSignal } from "solid-js";
import {
  Text,
  bindNodeRef,
  bindStyle,
  scene,
  type NodeHandle,
  type ViewStyle,
} from "@impasto/solid-renderer";
import { eventsOf } from "./element.tsx";
import { bindFocusRing } from "./focus.ts";
import { bindPressable } from "./pressable.ts";
import type { ButtonProps } from "./props.ts";
import { Role } from "./semantics.ts";
import { minTarget } from "./theme.ts";
import { useTheme } from "./use-theme.ts";

function faded(hex: string, blocked: boolean): string {
  if (!blocked) {
    return hex;
  }
  const body = hex.startsWith("#") ? hex.slice(1) : hex;
  return `#${body.slice(0, 6)}66`;
}

export function Button(props: ButtonProps): NodeHandle {
  const theme = useTheme();
  const [pressed, setPressed] = createSignal(false);
  const [focused, setFocused] = createSignal(false);
  const blocked = () => !!props.disabled || !!props.loading;
  const size = () => props.size ?? "regular";
  const variant = () => props.variant ?? "quiet";
  let node = 0;
  bindFocusRing(() => node, focused);
  const scale = () => theme().textScale;
  const padX = () =>
    (size() === "compact" ? theme().space.m : theme().space.l) * scale();
  const padY = () =>
    (size() === "compact" ? theme().space.s : theme().space.l) * scale();
  const target = () => minTarget(size(), theme().density) * scale();
  function face(): string {
    const color = pressed()
      ? variant() === "primary"
        ? theme().color.primaryContainer
        : theme().color.surfaceContainer
      : variant() === "primary"
        ? theme().color.accent
        : theme().color.surface;
    return faded(color, blocked());
  }
  function labelColor(): string {
    const color =
      variant() === "primary"
        ? pressed()
          ? theme().color.onPrimaryContainer
          : theme().color.onPrimary
        : theme().color.text;
    return faded(color, blocked());
  }
  const press = bindPressable({
    node: () => node,
    disabled: blocked,
    focused,
    setPressed,
    setFocused,
    onActivate: () => props.onClick(),
    events: props,
  });
  function writeSemantics(): void {
    if (node === 0) {
      return;
    }
    scene.setFocusable(node, !blocked());
    scene.setSemantics(
      node,
      Role.Button,
      props.accessibilityLabel ?? props.label,
      undefined,
      undefined,
      undefined,
      undefined,
      blocked() ? 1 : 0,
      undefined,
      undefined,
      props.accessibilityHint,
    );
  }
  createTrackedEffect(() => {
    writeSemantics();
  });
  return (
    <HostView
      {...eventsOf(props)}
      ref={(handle) => {
        if (handle === null) return;
        node = handle;
        bindStyle(handle, {
          get defaults(): ViewStyle {
            return {
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "center",
              gap: theme().space.s,
              minWidth: target(),
              minHeight: target(),
              paddingLeft: padX(),
              paddingRight: padX(),
              paddingTop: padY(),
              paddingBottom: padY(),
              backgroundColor: face(),
              borderRadius: theme().shape.full,
              color: labelColor(),
              fontSize: theme().typography.label.size,
              fontWeight: theme().typography.label.weight,
            };
          },
          get class() {
            return props.class;
          },
          get classList() {
            return props.classList;
          },
          get style() {
            return props.style;
          },
        });
        bindNodeRef(handle, () => props.ref);
        writeSemantics();
      }}
      onPress={press.onPress}
      onRelease={press.onRelease}
      onClick={press.onClick}
      onFocus={press.onFocus}
      onBlur={press.onBlur}
      onKey={press.onKey}
      onText={press.onText}
    >
      {props.icon}
      <Text
        class={props.labelProps?.class}
        classList={props.labelProps?.classList}
        style={props.labelProps?.style}
      >
        {props.label}
      </Text>
    </HostView>
  );
}
