import type { OpenUrlOptions, UrlListener } from "./types";
export function isLinkingHostAvailable() { return false; }
export function getInitialURL(): string | null { return null; }
export async function canOpenURL(_url: string): Promise<boolean> { return false; }
export async function openURL(_url: string, _options?: OpenUrlOptions): Promise<boolean> {
  throw Object.assign(new Error("Opening links requires a native host"), { code: "host-unavailable" });
}
export function watchUrls(_signal?: AbortSignal): ReadableStream<string> {
  return new ReadableStream({ start(controller) { controller.close(); } });
}
export function addEventListener(_type: "url", _listener: UrlListener): void {}
export function removeEventListener(_type: "url", _listener: UrlListener): void {}
export const Linking = Object.freeze({ getInitialURL, canOpenURL, openURL, watchUrls, addEventListener, removeEventListener });
