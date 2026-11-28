import { driver } from "./driver";
import type { ClipboardContent, ClipboardFormat } from "./types";
export type * from "./types";

export function getString(): string { return driver().getString(); }
export function setString(text: string): void { driver().setString(text); }
export function hasStrings(): Promise<boolean> { return driver().hasStrings(); }
export function hasImages(): Promise<boolean> { return driver().hasImages(); }
export function hasHtml(): Promise<boolean> { return driver().hasHtml(); }
export function availableFormats(): Promise<ClipboardFormat[]> { return driver().availableFormats(); }

export async function read(): Promise<ClipboardContent> {
  const { text, html, uri, imagePng } = await driver().read();
  return {
    text: text ?? undefined,
    html: html ?? undefined,
    uri: uri ?? undefined,
    imagePng: imagePng == null ? undefined : new Uint8Array(imagePng),
  };
}

export function write(content: ClipboardContent): Promise<void> {
  return driver().write({
    text: content.text,
    html: content.html,
    uri: content.uri,
    imagePng: content.imagePng,
  });
}

export function clear(): Promise<void> { return driver().clear(); }

export function watchChanges(signal?: AbortSignal): ReadableStream<readonly ClipboardFormat[]> {
  return driver().watchChanges(signal);
}

/** Preserve the synchronously callable clipboard surface used by existing applications. */
export const Clipboard = Object.freeze({ getString, setString });
