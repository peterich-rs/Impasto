import { createTrackedEffect } from "solid-js";
import { scene, type ViewEvents } from "@impasto/solid-renderer";

/** Shared press, keyboard, and disabled contract. Does not create a node. */
export function bindPressable(input: {
  node: () => number;
  disabled: () => boolean;
  focused: () => boolean;
  setPressed: (pressed: boolean) => void;
  setFocused: (focused: boolean) => void;
  onActivate: () => void;
  events: ViewEvents;
}): {
  onPress: () => void;
  onRelease: () => void;
  onClick: () => void;
  onFocus: () => void;
  onBlur: () => void;
  onKey: (key: string) => void;
  onText: (value: string) => void;
} {
  const blocked = () => input.disabled();
  function activate(): void {
    if (blocked()) {
      return;
    }
    input.onActivate();
  }
  createTrackedEffect(() => {
    const id = input.node();
    const off = blocked();
    if (id !== 0) scene.setFocusable(id, !off);
    if (!off) {
      return;
    }
    input.setPressed(false);
    if (input.focused()) {
      scene.blur();
    }
  });
  return {
    onPress: () => {
      if (!blocked()) input.setPressed(true);
      input.events.onPress?.();
    },
    onRelease: () => {
      input.setPressed(false);
      input.events.onRelease?.();
    },
    onClick: activate,
    onFocus: () => {
      input.setFocused(true);
      input.events.onFocus?.();
    },
    onBlur: () => {
      input.setFocused(false);
      input.setPressed(false);
      input.events.onBlur?.();
    },
    onKey: (key) => {
      if (key === "Enter" || key === " ") activate();
      input.events.onKey?.(key);
    },
    onText: (value) => {
      if (value === " ") activate();
      input.events.onText?.(value);
    },
  };
}
