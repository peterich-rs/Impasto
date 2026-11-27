import type { ConnectivityState, ConnectivityType } from "./types";
export function isConnectivityHostAvailable() { return false; }
export function getStatus() {
  const types: ConnectivityType[] = ["none"];
  return Promise.resolve({ types, isConnected: false, isExpensive: false, isConstrained: false });
}
export function watchStatus(signal?: AbortSignal): ReadableStream<ConnectivityState & { types: ConnectivityType[] }> {
  return new ReadableStream({
    async start(controller) {
      if (signal?.aborted) controller.error(signal.reason);
      else { controller.enqueue(await getStatus()); controller.close(); }
    },
  });
}
