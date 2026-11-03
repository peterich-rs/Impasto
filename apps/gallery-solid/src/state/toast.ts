import { createSignal } from "solid-js";
import { requestFrame } from "@impasto/solid-renderer";

const [open, setOpen] = createSignal(false);
const [text, setText] = createSignal("");

/** App-shell toast. Pages call `showToast`; `App` mounts the `Toast` host. */
export function showToast(message: string): void {
  setText(message);
  setOpen(true);
  requestFrame();
}

export function toastOpen(): boolean {
  return open();
}

export function toastText(): string {
  return text();
}

export function closeToast(): void {
  setOpen(false);
}
