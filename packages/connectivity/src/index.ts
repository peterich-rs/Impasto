import { driver } from "./driver";
import type { ConnectivityState } from "./types";
export type * from "./types";

export function getStatus(): Promise<ConnectivityState> { return driver().getStatus(); }
export function watchStatus(signal?: AbortSignal): ReadableStream<ConnectivityState> { return driver().watchStatus(signal); }

export function onStatusChange(listener: (state: ConnectivityState) => void): () => void {
  const cancel = new AbortController();
  const reader = watchStatus(cancel.signal).getReader();
  void (async () => {
    try {
      while (!cancel.signal.aborted) {
        const { done, value } = await reader.read();
        if (done) break;
        listener(value);
      }
    } catch (error) {
      if (!cancel.signal.aborted) console.error(error);
    } finally {
      reader.releaseLock();
    }
  })();
  return () => cancel.abort();
}
