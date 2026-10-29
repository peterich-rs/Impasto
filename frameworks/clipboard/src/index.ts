import * as host from "@uifw/clipboard/host";

export interface Clipboard {
  getString(): string;
  setString(text: string): void;
  addEventListener(type: string, listener: (e: unknown) => void): void;
  removeEventListener(type: string, listener: (e: unknown) => void): void;
}

export const Clipboard = host as unknown as Clipboard;
