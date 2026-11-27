// Browser and test environments without generated native bindings.
import { memoryClipboard } from "./memory";

const memory = memoryClipboard();
export const {
  getString, setString, hasStrings, hasImages, hasHtml, availableFormats, read, write, clear, watchChanges,
} = memory;
export function isClipboardHostAvailable(): boolean { return false; }
export const Clipboard = Object.freeze({ getString, setString });
