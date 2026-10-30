import { host } from "./host.ts";

type FocusHost = {
  focused?: () => number;
  isFocusable?: (node: number) => boolean;
  blur?: () => void;
  focus: (node: number) => void;
  firstChild: (node: number) => number;
  nextSibling: (node: number) => number;
};

function focusHost(): FocusHost {
  return host as FocusHost;
}

export function firstFocusable(id: number): number {
  const core = focusHost();
  if (id !== 0 && core.isFocusable?.(id)) {
    return id;
  }
  let child = core.firstChild(id);
  while (child !== 0) {
    const found = firstFocusable(child);
    if (found !== 0) {
      return found;
    }
    child = core.nextSibling(child);
  }
  return 0;
}

export function takeInitialFocus(scope: number, initialFocus?: number): void {
  const core = focusHost();
  if (typeof initialFocus === "number") {
    try {
      core.focus(initialFocus);
    } catch {
      // Unknown ids throw. An id outside the scope does not, and falls through.
    }
    if (core.focused?.() === initialFocus) {
      return;
    }
  }
  const target = firstFocusable(scope);
  if (target === 0) {
    return;
  }
  try {
    core.focus(target);
  } catch {
    // Leave the scope unfocused.
  }
}

export function restoreFocus(previous: number): void {
  const core = focusHost();
  if (previous === 0) {
    core.blur?.();
    return;
  }
  try {
    core.focus(previous);
  } catch {
    core.blur?.();
    return;
  }
  if (core.focused?.() !== previous) {
    core.blur?.();
  }
}
