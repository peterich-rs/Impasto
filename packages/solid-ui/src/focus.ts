import { createTrackedEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { host, onFrame, requestFrame, setStroke } from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Focus helpers for the script side.
 *
 * Tab order, nested scopes, and `pop_focus_scope` live in `core`
 * (`collect_focusable`, `push_focus_scope`, `pop_focus_scope`). The model
 * below states those invariants for tests that cannot run the Rust tree.
 * A scope with a single focusable node leaves Tab on that node.
 * A nested scope cycles inside itself.
 * After `pop_focus_scope`, `restoreFocus` returns to the node that was
 * focused outside the scope.
 */
export type FocusModel = {
  focused: number;
  scopes: number[][];
};

export function tabInScope(model: FocusModel): FocusModel {
  const scope = model.scopes[model.scopes.length - 1] ?? [];
  if (scope.length <= 1) {
    return model;
  }
  const index = Math.max(0, scope.indexOf(model.focused));
  const next = scope[(index + 1) % scope.length] ?? model.focused;
  return { ...model, focused: next };
}

export function popFocusModel(model: FocusModel, restore: number): FocusModel {
  return {
    focused: restore,
    scopes: model.scopes.slice(0, -1),
  };
}

export function useFocus(id: Accessor<number>): {
  focused: Accessor<boolean>;
  focus: () => void;
  blur: () => void;
  onFocus: () => void;
  onBlur: () => void;
} {
  const [focused, setFocused] = createSignal(false);
  return {
    focused,
    focus: () => {
      const node = id();
      if (node !== 0) {
        host.focus(node);
      }
    },
    blur: () => {
      host.blur();
    },
    onFocus: () => setFocused(true),
    onBlur: () => setFocused(false),
  };
}

/**
 * Focus on the next `onFrame`, not the frame that inserted the node.
 * The first delivery is skipped because `boot` runs listeners after flush.
 */
export function autoFocus(id: Accessor<number>): void {
  let skipped = false;
  const stop = onFrame(() => {
    if (!skipped) {
      skipped = true;
      requestFrame();
      return;
    }
    stop();
    const node = id();
    if (node === 0) {
      return;
    }
    try {
      host.focus(node);
    } catch {
      // Unknown ids throw. Leave focus unchanged.
    }
  });
  onCleanup(stop);
  requestFrame();
}

/** Stroke uses `theme.focusRing` only while focused, and only the stroke setter. */
export function bindFocusRing(id: Accessor<number>, focused: Accessor<boolean>): void {
  const theme = useTheme();
  createTrackedEffect(() => {
    const node = id();
    if (node === 0) {
      return;
    }
    const on = focused();
    const color = on ? theme().color.focusRing : theme().color.transparent;
    setStroke(node, on ? 2 : 0, colorBytes(color));
  });
}
