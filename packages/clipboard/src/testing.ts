import { installMock } from "./driver";
import { memoryClipboard } from "./memory";
import type { ClipboardContent } from "./types";

export function mockClipboard(initial?: ClipboardContent) {
  const memory = memoryClipboard(initial);
  return { simulateExternalCopy: memory.simulateExternalCopy, restore: installMock(memory) };
}
