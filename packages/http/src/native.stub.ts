import type { NativeHttpEvent, NativeHttpRequest } from "@impasto/http/native";

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
export const isHttpHostAvailable = () => false;
