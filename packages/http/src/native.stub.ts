import type { NativeHttpEvent, NativeHttpRequest } from "./native.d.ts";

export function send(
  _request: NativeHttpRequest,
  _signal?: AbortSignal,
): ReadableStream<NativeHttpEvent> {
  return new ReadableStream<NativeHttpEvent>({
    start(controller) {
      controller.error(new Error("[impasto:http:network] Http is not linked"));
    },
  });
}

export const Http = { send };
