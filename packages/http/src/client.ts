import type { NativeHttpEvent, NativeHttpRequest } from "@impasto/http/native";
import { FormData } from "./form-data.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export interface HttpTransport {
  send(
    request: NativeHttpRequest,
    signal?: AbortSignal,
  ): ReadableStream<NativeHttpEvent>;
}

export type RedirectMode = "follow" | "error" | "manual";

export type HttpBodyInput =
  | string
  | Uint8Array
  | ArrayBuffer
  | ArrayBufferView
  | URLSearchParams
  | FormData
  | ReadableStream<Uint8Array>
  | null
  | undefined;

export interface HttpRequestOptions {
  method?: string;
  headers?: HeadersInit;
  query?: Record<string, string | number | boolean | null | undefined> | URLSearchParams;
  body?: HttpBodyInput;
  json?: unknown;
  timeoutMs?: number;
  redirect?: RedirectMode;
  maxBodyBytes?: number;
  signal?: AbortSignal;
}

export interface HttpClientDefaults {
  baseUrl?: string;
  headers?: HeadersInit;
  timeoutMs?: number;
  redirect?: RedirectMode;
  maxBodyBytes?: number;
}

export interface HttpResponseData {
  readonly status: number;
  readonly ok: boolean;
  readonly url: string;
  readonly redirected: boolean;
  readonly headers: Headers;
  readonly body: ReadableStream<Uint8Array> | null;
  text(): Promise<string>;
  json<T = unknown>(): Promise<T>;
  bytes(): Promise<Uint8Array>;
  arrayBuffer(): Promise<ArrayBuffer>;
}

export type HttpErrorCode =
  | "network"
  | "timeout"
  | "canceled"
  | "too-large"
  | "bad-url"
  | "http-status";

export class HttpError extends Error {
  readonly code: HttpErrorCode;
  readonly status?: number;
  readonly url?: string;
  readonly headers?: Headers;

  constructor(
    code: HttpErrorCode,
    message: string,
    detail?: { status?: number; url?: string; headers?: Headers },
  ) {
    super(message);
    this.name = "HttpError";
    this.code = code;
    this.status = detail?.status;
    this.url = detail?.url;
    this.headers = detail?.headers;
  }
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();
const WIRE_PREFIX = /^\[impasto:http:([a-z-]+)\]\s*(.*)$/;

export function toUint8Array(data: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

export function createNamedError(name: string, message: string): Error {
  if (typeof DOMException === "function") {
    return new DOMException(message, name);
  }
  const err = new Error(message);
  err.name = name;
  return err;
}

export function mapNativeError(error: unknown, fetchStyle = false): Error {
  if (error instanceof Error) {
    if (error.name === "AbortError" || error.name === "TimeoutError") {
      return error;
    }
    const match = WIRE_PREFIX.exec(error.message);
    if (match) {
      const code = match[1] as HttpErrorCode;
      const message = match[2] || error.message;
      if (code === "canceled") {
        return createNamedError("AbortError", message);
      }
      if (code === "timeout") {
        return createNamedError("TimeoutError", message);
      }
      if (fetchStyle) {
        return new TypeError(message);
      }
      return new HttpError(code, message);
    }
    return fetchStyle && !(error instanceof TypeError)
      ? new TypeError(error.message)
      : error;
  }
  const message = String(error);
  return fetchStyle ? new TypeError(message) : new HttpError("network", message);
}

export async function readAllBytes(
  stream: ReadableStream<Uint8Array> | null,
): Promise<Uint8Array> {
  if (!stream) return new Uint8Array(0);
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const step = await reader.read();
      if (step.done) break;
      const bytes = toUint8Array(step.value);
      chunks.push(bytes);
      total += bytes.byteLength;
    }
  } finally {
    reader.releaseLock();
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return out;
}

export async function encodeRequestBody(
  body: HttpBodyInput,
  headers: Headers,
): Promise<Uint8Array | undefined> {
  if (body == null) return undefined;
  if (typeof body === "string") {
    if (!headers.has("content-type")) {
      headers.set("content-type", "text/plain;charset=UTF-8");
    }
    return textEncoder.encode(body);
  }
  if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
    return toUint8Array(body);
  }
  if (body instanceof URLSearchParams) {
    if (!headers.has("content-type")) {
      headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
    }
    return textEncoder.encode(body.toString());
  }
  if (body instanceof FormData) {
    const serialized = await body.serializeMultipart();
    headers.set("content-type", serialized.contentType);
    return serialized.body;
  }
  if (body instanceof ReadableStream) {
    return readAllBytes(body);
  }
  throw new TypeError("Unsupported HTTP request body");
}

export interface StreamHead {
  status: number;
  url: string;
  redirected: boolean;
  headers: Headers;
  body: ReadableStream<Uint8Array> | null;
}

export async function executeNativeStream(
  transport: HttpTransport,
  request: NativeHttpRequest,
  options?: {
    signal?: AbortSignal;
    emptyBody?: boolean;
    fetchStyle?: boolean;
  },
): Promise<StreamHead> {
  const signal = options?.signal;
  const fetchStyle = options?.fetchStyle ?? false;
  if (signal?.aborted) {
    throw signal.reason !== undefined
      ? (signal.reason as Error)
      : createNamedError("AbortError", "The operation was aborted.");
  }

  let nativeStream: ReadableStream<NativeHttpEvent>;
  try {
    nativeStream = transport.send(request, signal);
  } catch (err) {
    throw mapNativeError(err, fetchStyle);
  }

  const reader = nativeStream.getReader();
  let first: ReadableStreamReadResult<NativeHttpEvent>;
  try {
    first = await reader.read();
  } catch (err) {
    reader.releaseLock();
    throw mapNativeError(err, fetchStyle);
  }

  if (first.done || first.value.type !== "head") {
    reader.releaseLock();
    throw fetchStyle
      ? new TypeError("HTTP response stream ended before head")
      : new HttpError("network", "HTTP response stream ended before head");
  }

  const head = first.value;
  const headers = new Headers(head.headers);
  const omitBody =
    options?.emptyBody === true ||
    head.status === 204 ||
    head.status === 205 ||
    head.status === 304;

  if (omitBody) {
    void reader.cancel().catch(() => {});
    reader.releaseLock();
    return {
      status: head.status,
      url: head.url,
      redirected: head.redirected,
      headers,
      body: null,
    };
  }

  const body = new ReadableStream<Uint8Array>(
    {
      async pull(controller) {
        try {
          for (;;) {
            const step = await reader.read();
            if (step.done) {
              controller.close();
              reader.releaseLock();
              return;
            }
            if (step.value.type === "chunk") {
              controller.enqueue(toUint8Array(step.value.data));
              return;
            }
          }
        } catch (err) {
          controller.error(mapNativeError(err, fetchStyle));
          reader.releaseLock();
        }
      },
      async cancel(reason) {
        try {
          await reader.cancel(reason);
        } finally {
          reader.releaseLock();
        }
      },
    },
    { highWaterMark: 0 },
  );

  return {
    status: head.status,
    url: head.url,
    redirected: head.redirected,
    headers,
    body,
  };
}

function buildUrl(
  input: string,
  baseUrl?: string,
  query?: HttpRequestOptions["query"],
): string {
  const url = baseUrl ? new URL(input, baseUrl) : new URL(input);
  if (query instanceof URLSearchParams) {
    for (const [k, v] of query.entries()) {
      url.searchParams.append(k, v);
    }
  } else if (query) {
    for (const [k, v] of Object.entries(query)) {
      if (v != null) {
        url.searchParams.append(k, String(v));
      }
    }
  }
  return url.toString();
}

export class HttpClient {
  readonly #transport: HttpTransport;
  readonly #defaults: HttpClientDefaults;

  constructor(transport: HttpTransport, defaults: HttpClientDefaults = {}) {
    this.#transport = transport;
    this.#defaults = defaults;
  }

  async send(input: string, options: HttpRequestOptions = {}): Promise<HttpResponseData> {
    const url = buildUrl(input, this.#defaults.baseUrl, options.query);
    const method = (options.method ?? "GET").toUpperCase();
    const headers = new Headers(this.#defaults.headers);
    if (options.headers) {
      for (const [k, v] of new Headers(options.headers).entries()) {
        headers.set(k, v);
      }
    }

    let bodyBytes: Uint8Array | undefined;
    if (options.json !== undefined) {
      if (options.body !== undefined) {
        throw new TypeError("Cannot pass both `json` and `body` in HTTP request options");
      }
      if (!headers.has("content-type")) {
        headers.set("content-type", "application/json");
      }
      bodyBytes = textEncoder.encode(JSON.stringify(options.json));
    } else {
      bodyBytes = await encodeRequestBody(options.body, headers);
    }

    if ((method === "GET" || method === "HEAD") && bodyBytes && bodyBytes.byteLength > 0) {
      throw new TypeError(`HTTP ${method} request cannot include a body`);
    }

    const head = await executeNativeStream(
      this.#transport,
      {
        method,
        url,
        headers: headers.toPairs(),
        body: bodyBytes,
        timeoutMs: options.timeoutMs ?? this.#defaults.timeoutMs,
        redirect: options.redirect ?? this.#defaults.redirect ?? "follow",
        maxBodyBytes: options.maxBodyBytes ?? this.#defaults.maxBodyBytes,
      },
      {
        signal: options.signal,
        emptyBody: method === "HEAD",
        fetchStyle: false,
      },
    );

    let consumed = false;
    const consumeBytes = async (): Promise<Uint8Array> => {
      if (consumed) {
        throw new TypeError("HTTP response body has already been consumed");
      }
      consumed = true;
      return readAllBytes(head.body);
    };

    return {
      status: head.status,
      ok: head.status >= 200 && head.status < 300,
      url: head.url,
      redirected: head.redirected,
      headers: head.headers,
      body: head.body,
      async text() {
        return textDecoder.decode(await consumeBytes());
      },
      async json<T = unknown>() {
        return JSON.parse(textDecoder.decode(await consumeBytes())) as T;
      },
      bytes: consumeBytes,
      async arrayBuffer() {
        const bytes = await consumeBytes();
        const copy = new Uint8Array(bytes.byteLength);
        copy.set(bytes);
        return copy.buffer;
      },
    };
  }

  async request(input: string, options: HttpRequestOptions = {}): Promise<HttpResponseData> {
    const response = await this.send(input, options);
    if (!response.ok) {
      await response.body?.cancel().catch(() => {});
      throw new HttpError(
        "http-status",
        `HTTP ${response.status} for ${response.url}`,
        {
          status: response.status,
          url: response.url,
          headers: response.headers,
        },
      );
    }
    return response;
  }

  get(input: string, options?: Omit<HttpRequestOptions, "method" | "body" | "json">) {
    return this.request(input, { ...options, method: "GET" });
  }

  head(input: string, options?: Omit<HttpRequestOptions, "method" | "body" | "json">) {
    return this.request(input, { ...options, method: "HEAD" });
  }

  post(input: string, options?: Omit<HttpRequestOptions, "method">) {
    return this.request(input, { ...options, method: "POST" });
  }

  put(input: string, options?: Omit<HttpRequestOptions, "method">) {
    return this.request(input, { ...options, method: "PUT" });
  }

  patch(input: string, options?: Omit<HttpRequestOptions, "method">) {
    return this.request(input, { ...options, method: "PATCH" });
  }

  delete(input: string, options?: Omit<HttpRequestOptions, "method">) {
    return this.request(input, { ...options, method: "DELETE" });
  }
}

export function createClient(
  transport: HttpTransport,
  defaults?: HttpClientDefaults,
): HttpClient {
  return new HttpClient(transport, defaults);
}

/**
 * Coordinates sequential latest-wins async operations by aborting the previous
 * request whenever a new `run` starts or `abort()` is called.
 */
export function createLatest<Args extends unknown[], Result>(
  runTask: (signal: AbortSignal, ...args: Args) => Promise<Result>,
): {
  run: (...args: Args) => Promise<Result>;
  abort: () => void;
} {
  let current: AbortController | null = null;
  return {
    run(...args) {
      current?.abort();
      const controller = new AbortController();
      current = controller;
      return runTask(controller.signal, ...args);
    },
    abort() {
      current?.abort();
      current = null;
    },
  };
}
