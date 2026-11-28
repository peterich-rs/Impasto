import * as native from "@impasto/http/native";
import {
  createClient as createClientWithTransport,
  createLatest,
  HttpClient,
  HttpError,
  type HttpBodyInput,
  type HttpClientDefaults,
  type HttpErrorCode,
  type HttpRequestOptions,
  type HttpResponseData,
  type HttpTransport,
  type RedirectMode,
} from "./client.ts";
import { FormData, type FormDataEntryValue } from "./form-data.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export {
  createLatest,
  FormData,
  Headers,
  HttpClient,
  HttpError,
};
export type {
  FormDataEntryValue,
  HeadersInit,
  HttpBodyInput,
  HttpClientDefaults,
  HttpErrorCode,
  HttpRequestOptions,
  HttpResponseData,
  HttpTransport,
  RedirectMode,
};

function nativeTransport(defaults: HttpClientDefaults): HttpTransport {
  let configured: ReturnType<typeof native.configureClient> | undefined;
  const hasConfiguration = Boolean(defaults.cookieJar || defaults.proxy || defaults.security);
  function configure() {
    return configured ??= native.configureClient({
      cookieJar: defaults.cookieJar,
      proxy: defaults.proxy && { ...defaults.proxy, bypass: defaults.proxy.bypass?.slice() },
      pins: defaults.security?.certificatePins && Object.entries(defaults.security.certificatePins)
        .map(([host, pins]) => [host, pins.slice()]),
    });
  }
  function withClient(
    start: (id: number) => ReadableStream<native.NativeHttpEvent>,
    signal?: AbortSignal,
  ): ReadableStream<native.NativeHttpEvent> {
    if (!hasConfiguration) return start(0);
    let reader: ReadableStreamDefaultReader<native.NativeHttpEvent> | undefined;
    let stopped = false;
    return new ReadableStream({
      async pull(controller) {
        try {
          if (!reader) {
            const client = await configure();
            if (stopped) return;
            if (signal?.aborted) throw signal.reason;
            reader = start(client.id()).getReader();
          }
          const item = await reader.read();
          if (stopped) return;
          if (item.done) {
            reader.releaseLock();
            reader = undefined;
            controller.close();
          } else controller.enqueue(item.value);
        } catch (error) {
          if (stopped) return;
          reader?.releaseLock();
          reader = undefined;
          controller.error(error);
        }
      },
      async cancel(reason) {
        stopped = true;
        try { await reader?.cancel(reason); }
        finally { reader?.releaseLock(); reader = undefined; }
      },
    });
  }
  return {
    send(request, signal) {
      return hasConfiguration
        ? withClient(id => native.sendConfigured(id, request, signal), signal)
        : native.send(request, signal);
    },
    download(request, { destinationPath, scope, resume, overwrite }, signal) {
      return withClient(id => native.download(id, request, { destinationPath, scope, resume, overwrite }, signal), signal);
    },
    upload(request, { path, scope, contentType }, signal) {
      return withClient(id => native.upload(id, request, { path, scope, contentType }, signal), signal);
    },
    cookies: defaults.cookieJar && {
      async get(url) {
        const entries = await (await configure()).getCookies(url);
        return entries.map(entry => ({ ...entry, expiresAtMs: entry.expiresAtMs ?? null }));
      },
      async set(url, value) { await (await configure()).setCookie(url, value); },
      async clear() { await (await configure()).clearCookies(); },
    },
  };
}

export function createClient(defaults?: HttpClientDefaults): HttpClient;
export function createClient(transport: HttpTransport, defaults?: HttpClientDefaults): HttpClient;
export function createClient(
  defaults: HttpClientDefaults | HttpTransport = {},
  transportDefaults?: HttpClientDefaults,
): HttpClient {
  return "send" in defaults
    ? createClientWithTransport(defaults, transportDefaults)
    : createClientWithTransport(nativeTransport(defaults), defaults);
}

export const http: HttpClient = createClient();
export default http;

export type * from "./middleware.ts";
