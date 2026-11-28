import * as native from "@impasto/http/native";
import type { NativeHttpEvent, NativeHttpRequest } from "@impasto/http/native";
import { exists, openWriter } from "@impasto/file-system";
import { createClient, type HttpClientDefaults, type HttpErrorCode, type HttpTransport } from "./client.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export interface MockHttpResponse {
  readonly status?: number;
  readonly headers?: HeadersInit;
  readonly chunks?: readonly (string | Uint8Array)[];
  readonly error?: HttpErrorCode;
}

/** Creates an isolated HTTP client with a scripted response for each request. */
export function mockHttp(responses: readonly MockHttpResponse[], defaults?: HttpClientDefaults) {
  if (native.isHttpHostAvailable()) {
    throw new Error("[impasto:http:io] HTTP mocks require a test host");
  }
  const pending = [...responses];
  const requests: NativeHttpRequest[] = [];
  const encoder = new TextEncoder();
  function* events(request: NativeHttpRequest): IterableIterator<NativeHttpEvent> {
    requests.push(request);
    const response = pending.shift();
    if (!response || response.error) {
      throw new Error(`[impasto:http:${response?.error ?? "network"}] Mock HTTP failure`);
    }
    const chunks = (response.chunks ?? []).map(chunk => typeof chunk === "string" ? encoder.encode(chunk) : chunk);
    const total = chunks.reduce((length, chunk) => length + chunk.byteLength, 0);
    yield { type: "head", status: response.status ?? 200, url: request.url, redirected: false,
      headers: new Headers(response.headers).toPairs() };
    let loaded = 0;
    for (const data of chunks) {
      yield { type: "chunk", data };
      loaded += data.byteLength;
      yield { type: "downloadProgress", loaded, total };
    }
  }

  function stream(source: AsyncIterator<NativeHttpEvent> | Iterator<NativeHttpEvent>, signal?: AbortSignal) {
    return new ReadableStream<NativeHttpEvent>({
      async pull(controller) {
        try {
          if (signal?.aborted) throw signal.reason;
          const result = await source.next();
          if (result.done) controller.close();
          else controller.enqueue(result.value);
        } catch (error) {
          await source.return?.();
          controller.error(error);
        }
      },
      async cancel() { await source.return?.(); },
    }, { highWaterMark: 0 });
  }

  const transport: HttpTransport = {
    send: (request, signal) => stream(events(request), signal),
    download(request, file, signal) {
      const source = async function* (): AsyncIterableIterator<NativeHttpEvent> {
        let writer: Awaited<ReturnType<typeof openWriter>> | undefined;
        let size = 0;
        try {
          for (const event of events(request)) {
            if (event.type === "head" && event.status >= 200 && event.status < 300) {
              const scope = file.scope ?? "cache";
              if (file.overwrite === false && await exists(file.destinationPath, scope)) {
                throw new Error("[impasto:http:io] Download destination already exists");
              }
              writer = await openWriter(file.destinationPath, { scope, atomic: true });
            }
            if (event.type === "chunk") {
              if (writer) {
                const bytes = event.data instanceof Uint8Array ? event.data : new Uint8Array(event.data);
                await writer.write(bytes);
                size += bytes.byteLength;
              }
            } else yield event;
          }
          if (writer) {
            await writer.commit();
            writer = undefined;
            yield { type: "file", bytesWritten: size, resumed: false, path: file.destinationPath };
          }
        } finally { await writer?.abort(); }
      };
      return stream(source(), signal);
    },
  };
  return { client: createClient(transport, defaults), requests, enqueue: (response: MockHttpResponse) => pending.push(response) };
}
