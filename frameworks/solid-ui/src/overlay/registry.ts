import { onBack } from "@uifw/solid-renderer";

export type OverlayEntry = {
  id: number;
  modal: boolean;
  close: () => void;
};

let nextId = 1;
const entries: OverlayEntry[] = [];
let releaseBack: (() => void) | null = null;

export function overlayEntries(): readonly OverlayEntry[] {
  return entries;
}

/** Dialog and Sheet pass `modal: true`. Toast and Tooltip do not register. */
export function registerOverlay(entry: { modal: boolean; close: () => void }): () => void {
  const spec: OverlayEntry = { id: nextId, modal: entry.modal, close: entry.close };
  nextId += 1;
  entries.push(spec);
  if (releaseBack === null) {
    releaseBack = onBack(300, closeTopModal);
  }
  return () => {
    const index = entries.findIndex((item) => item.id === spec.id);
    if (index >= 0) {
      entries.splice(index, 1);
    }
  };
}

/** Closes the topmost modal. Toast and Tooltip are not in this stack. */
export function closeTopModal(): boolean {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (entry?.modal) {
      entry.close();
      return true;
    }
  }
  return false;
}

export function resetOverlaysForTests(): void {
  entries.length = 0;
  nextId = 1;
}
