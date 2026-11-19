import type { Clipboard as ClipboardApi } from "./native.js";

let text = "";

export function getString(): string {
  return text;
}

export function setString(value: string): void {
  text = value;
}

export const Clipboard: ClipboardApi = Object.freeze({
  getString,
  setString,
});
