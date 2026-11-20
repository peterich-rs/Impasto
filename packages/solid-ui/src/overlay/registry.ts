import { onBack } from "@impasto/solid-renderer";
export type OverlayEntry = {
  id: number;
  modal: boolean;
  close: () => void;
};
let nextId = 1;
const entries: (OverlayEntry & { releaseBack?: () => void })[] = [];
export function overlayEntries(): readonly OverlayEntry[] {
  return entries;
}

/** Dialog and Sheet pass `modal: true`. Toast and Tooltip do not register. */
export function registerOverlay(entry: {
  modal: boolean;
  close: () => void;
}): () => void {
  const spec: (typeof entries)[number] = {
    id: nextId,
    modal: entry.modal,
    close: entry.close,
    releaseBack: entry.modal ? onBack(300, () => {
      entry.close();
      return true;
    }) : undefined,
  };
  nextId += 1;
  entries.push(spec);
  return () => {
    const index = entries.findIndex((item) => item.id === spec.id);
    if (index >= 0) {
      entries.splice(index, 1);
    }
    spec.releaseBack?.();
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
  for (const entry of entries) entry.releaseBack?.();
  entries.length = 0;
  nextId = 1;
}
