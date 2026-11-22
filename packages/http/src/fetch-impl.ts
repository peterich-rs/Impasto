import {
  encodeRequestBody,
  executeNativeStream,
  readAllBytes,
  toUint8Array,
  type HttpBodyInput,
  type HttpTransport,
  type RedirectMode,
} from "./client.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export interface RequestInit {
  method?: string;
  headers?: HeadersInit;
  body?: HttpBodyInput;
  redirect?: RedirectMode;
  signal?: AbortSignal | null;
}

export type RequestInfo = string | URL | Request;

const textDecoder = new TextDecoder();

const REASON_PHRASES: Record<number, string> = {
  200: "OK",
  201: "Created",
  202: "Accepted",
  204: "No Content",
  206: "Partial Content",
  301: "Moved Permanently",
  302: "Found",
  304: "Not Modified",
  307: "Temporary Redirect",
  308: "Permanent Redirect",
  400: "Bad Request",
  401: "Unauthorized",
  403: "Forbidden",
  404: "Not Found",
  405: "Method Not Allowed",
  408: "Request Timeout",
  409: "Conflict",
  413: "Payload Too Large",
  429: "Too Many Requests",
  500: "Internal Server Error",
  502: "Bad Gateway",
  503: "Service Unavailable",
  504: "Gateway Timeout",
};

export class Request {
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  readonly redirect: RedirectMode;
  readonly signal: AbortSignal | undefined;
  readonly #bodyInput: HttpBodyInput;

  constructor(input: RequestInfo, init: RequestInit = {}) {
    if (input instanceof Request) {
      this.url = input.url;
      this.method = (init.method ?? input.method).toUpperCase();
      this.headers = new Headers(init.headers ?? input.headers);
      this.redirect = init.redirect ?? input.redirect;
      this.signal = init.signal === undefined ? input.signal : (init.signal ?? undefined);
      this.#bodyInput = init.body === undefined ? input.#bodyInput : init.body;
    } else {
      this.url = input instanceof URL ? input.toString() : String(input);
      this.method = (init.method ?? "GET").toUpperCase();
      this.headers = new Headers(init.headers);
      this.redirect = init.redirect ?? "follow";
      this.signal = init.signal ?? undefined;
      this.#bodyInput = init.body;
    }

    if ((this.method === "GET" || this.method === "HEAD") && this.#bodyInput != null) {
      throw new TypeError(`Request with ${this.method} method cannot have a body.`);
    }
  }

  getBodyInput(): HttpBodyInput {
    return this.#bodyInput;
  }

  clone(): Request {
    return new Request(this);
  }
}

export interface ResponseInit {
  status?: number;
  statusText?: string;
  headers?: HeadersInit;
}

export class Response {
  readonly status: number;
  readonly statusText: string;
  readonly ok: boolean;
  readonly url: string;
  readonly redirected: boolean;
  readonly headers: Headers;
  #body: ReadableStream<Uint8Array> | null;
  #bodyUsed = false;

  constructor(
    body?: HttpBodyInput,
    init: ResponseInit & { url?: string; redirected?: boolean } = {},
  ) {
    const status = init.status ?? 200;
    if (status < 200 || status > 599) {
      throw new RangeError(`Invalid HTTP status: ${status}`);
    }
    this.status = status;
    this.statusText = init.statusText ?? REASON_PHRASES[status] ?? "";
    this.ok = status >= 200 && status < 300;
    this.url = init.url ?? "";
    this.redirected = init.redirected ?? false;
    this.headers = new Headers(init.headers);

    if (body == null || status === 204 || status === 205 || status === 304) {
      this.#body = null;
    } else if (body instanceof ReadableStream) {
      this.#body = body;
    } else if (typeof body === "string") {
      if (!this.headers.has("content-type")) {
        this.headers.set("content-type", "text/plain;charset=UTF-8");
      }
      const encoded = new TextEncoder().encode(body);
      this.#body = singleChunkStream(encoded);
    } else if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
      this.#body = singleChunkStream(toUint8Array(body));
    } else if (body instanceof URLSearchParams) {
      if (!this.headers.has("content-type")) {
        this.headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
      this.#body = singleChunkStream(new TextEncoder().encode(body.toString()));
    } else {
      throw new TypeError("Unsupported Response body initializer");
    }
  }

  static json(data: unknown, init: ResponseInit = {}): Response {
    const headers = new Headers(init.headers);
    if (!headers.has("content-type")) {
      headers.set("content-type", "application/json");
    }
    return new Response(JSON.stringify(data), { ...init, headers });
  }

  get body(): ReadableStream<Uint8Array> | null {
    return this.#body;
  }

  get bodyUsed(): boolean {
    return this.#bodyUsed;
  }

  async #consume(): Promise<Uint8Array> {
    if (this.#bodyUsed) {
      throw new TypeError("Response body already used");
    }
    this.#bodyUsed = true;
    return readAllBytes(this.#body);
  }

  async text(): Promise<string> {
    return textDecoder.decode(await this.#consume());
  }

  async json<T = unknown>(): Promise<T> {
    return JSON.parse(textDecoder.decode(await this.#consume())) as T;
  }

  async bytes(): Promise<Uint8Array> {
    return this.#consume();
  }

  async arrayBuffer(): Promise<ArrayBuffer> {
    const bytes = await this.#consume();
    const copy = new Uint8Array(bytes.byteLength);
    copy.set(bytes);
    return copy.buffer;
  }

  clone(): Response {
    if (this.#bodyUsed) {
      throw new TypeError("Cannot clone a Response whose body has already been used");
    }
    if (!this.#body) {
      return new Response(null, {
        status: this.status,
        statusText: this.statusText,
        headers: this.headers,
        url: this.url,
        redirected: this.redirected,
      });
    }
    const [left, right] = this.#body.tee();
    this.#body = left;
    return new Response(right, {
      status: this.status,
      statusText: this.statusText,
      headers: this.headers,
      url: this.url,
      redirected: this.redirected,
    });
  }
}

function singleChunkStream(chunk: Uint8Array): ReadableStream<Uint8Array> {
  let done = false;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      if (!done && chunk.byteLength > 0) {
        done = true;
        controller.enqueue(chunk);
      }
      controller.close();
    },
  });
}

export function createFetch(transport: HttpTransport) {
  return async function fetch(
    input: RequestInfo,
    init?: RequestInit,
  ): Promise<Response> {
    const request = input instanceof Request && !init ? input : new Request(input, init);
    const headers = new Headers(request.headers);
    const bodyBytes = await encodeRequestBody(request.getBodyInput(), headers);

    const head = await executeNativeStream(
      transport,
      {
        method: request.method,
        url: request.url,
        headers: headers.toPairs(),
        body: bodyBytes,
        redirect: request.redirect,
      },
      {
        signal: request.signal,
        emptyBody: request.method === "HEAD",
        fetchStyle: true,
      },
    );

    return new Response(head.body, {
      status: head.status,
      headers: head.headers,
      url: head.url,
      redirected: head.redirected,
    });
  };
}
