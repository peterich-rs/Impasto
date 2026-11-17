import * as scene from "./clipboard.ts";

export interface Clipboard {
  getString(): string;
  setString(text: string): void;
  addEventListener(type: string, listener: (e: unknown) => void): void;
  removeEventListener(type: string, listener: (e: unknown) => void): void;
}

export const Clipboard = scene as unknown as Clipboard;
