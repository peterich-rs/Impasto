import type { NativeWebSocketEvent, NativeWebSocketHandle } from "@impasto/websocket/native";

export function open(_url: string, _protocols?: string[]): NativeWebSocketHandle {
  throw new Error("WebSocket is not linked");
}

export function events(
  _id: number,
  _signal?: AbortSignal,
): ReadableStream<NativeWebSocketEvent> {
  return new ReadableStream<NativeWebSocketEvent>({
    start(controller) {
      controller.error(new Error("WebSocket is not linked"));
    },
  });
}

export const WebSocket = { open, events };

export function isWebsocketHostAvailable(): boolean { return false; }

export function watchOnline(_signal?: AbortSignal): ReadableStream<{ isConnected: boolean }> {
  return new ReadableStream({ start(controller) { controller.close(); } });
}
