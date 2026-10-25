import { host } from "@uifw/solid-renderer";
import { coreHost } from "./host-ext.ts";

export function moveChildren(from: number, to: number, skip: ReadonlySet<number> = new Set()): void {
  const kids: number[] = [];
  let child = host.firstChild(from);
  while (child !== 0) {
    const next = host.nextSibling(child);
    if (!skip.has(child)) {
      kids.push(child);
    }
    child = next;
  }
  for (const id of kids) {
    host.insert(to, id, 0);
  }
}

export function firstFocusable(id: number): number {
  if (id !== 0 && coreHost.isFocusable(id)) {
    return id;
  }
  let child = host.firstChild(id);
  while (child !== 0) {
    const found = firstFocusable(child);
    if (found !== 0) {
      return found;
    }
    child = host.nextSibling(child);
  }
  return 0;
}

export function takeInitialFocus(scope: number, initialFocus?: number): void {
  if (typeof initialFocus === "number") {
    try {
      host.focus(initialFocus);
    } catch {
      // Unknown ids throw. An id outside the scope does not, and falls through.
    }
    if (coreHost.focused() === initialFocus) {
      return;
    }
  }
  const target = firstFocusable(scope);
  if (target === 0) {
    return;
  }
  try {
    host.focus(target);
  } catch {
    // Leave the scope unfocused.
  }
}

export function restoreFocus(previous: number): void {
  if (previous === 0) {
    coreHost.blur();
    return;
  }
  try {
    host.focus(previous);
  } catch {
    coreHost.blur();
    return;
  }
  if (coreHost.focused() !== previous) {
    coreHost.blur();
  }
}
