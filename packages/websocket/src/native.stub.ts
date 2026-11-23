import type { NativeWebSocketEvent, NativeWebSocketHandle } from "./native.d.ts";

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
