import type { NativeWebSocketEvent, NativeWebSocketHandle } from "./native.js";

export const READY_STATE_CONNECTING = 0;
export const READY_STATE_OPEN = 1;
export const READY_STATE_CLOSING = 2;
export const READY_STATE_CLOSED = 3;

export type WebSocketMessage = string | Uint8Array;

export interface WebSocketCloseInfo {
  readonly code: number;
  readonly reason: string;
  readonly wasClean: boolean;
}

export interface ConnectOptions {
  protocols?: string | string[];
  signal?: AbortSignal;
}

export interface WebSocketTransport {
  open(url: string, protocols?: string[]): NativeWebSocketHandle;
  events(id: number, signal?: AbortSignal): ReadableStream<NativeWebSocketEvent>;
}

export function toUint8Array(data: ArrayBuffer | ArrayBufferView): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

function abortError(reason?: unknown): Error {
  if (reason instanceof Error) return reason;
  if (typeof DOMException === "function") {
    return new DOMException("The operation was aborted.", "AbortError");
  }
  return Object.assign(new Error("The operation was aborted."), { name: "AbortError" });
}

export function normalizeProtocols(protocols?: string | string[]): string[] | undefined {
  if (protocols === undefined) return undefined;
  const list = Array.isArray(protocols) ? protocols : [protocols];
  const seen = new Set<string>();
  for (const proto of list) {
    if (seen.has(proto)) {
      throw new SyntaxError(`Duplicate WebSocket subprotocol: ${proto}`);
    }
    seen.add(proto);
  }
  return list;
}

export class WebSocketClient {
  readonly url: string;
  readonly #handle: NativeWebSocketHandle;
  readonly #reader: ReadableStreamDefaultReader<NativeWebSocketEvent>;
  readonly #bufferedAfterClose = 0;
  #protocol = "";
  #readyState = READY_STATE_CONNECTING;
  #closedInfo: WebSocketCloseInfo | null = null;
  #handleReleased = false;
  #inFlightRead: Promise<NativeWebSocketEvent | null> | null = null;
  #drainingClose = false;
  #closedResolve!: (info: WebSocketCloseInfo) => void;
  readonly closed: Promise<WebSocketCloseInfo>;

  constructor(
    url: string,
    handle: NativeWebSocketHandle,
    stream: ReadableStream<NativeWebSocketEvent>,
  ) {
    this.url = url;
    this.#handle = handle;
    this.#reader = stream.getReader();
    this.closed = new Promise<WebSocketCloseInfo>((resolve) => {
      this.#closedResolve = resolve;
    });
  }

  get readyState(): number {
    if (this.#handleReleased) return this.#readyState;
    try {
      return this.#handle.call("readyState");
    } catch {
      return this.#readyState;
    }
  }

  get protocol(): string {
    if (this.#handleReleased) return this.#protocol;
    try {
      return this.#handle.call("protocol");
    } catch {
      return this.#protocol;
    }
  }

  get bufferedAmount(): number {
    if (this.#handleReleased) return this.#bufferedAfterClose;
    try {
      return this.#handle.call("bufferedAmount");
    } catch {
      return this.#bufferedAfterClose;
    }
  }

  send(data: string | ArrayBuffer | ArrayBufferView): number {
    if (typeof data === "string") {
      return this.sendText(data);
    }
    return this.sendBinary(toUint8Array(data));
  }

  sendText(text: string): number {
    return this.#handle.call("sendText", text);
  }

  sendBinary(data: ArrayBuffer | ArrayBufferView): number {
    return this.#handle.call("sendBinary", toUint8Array(data));
  }

  async close(code?: number, reason?: string): Promise<WebSocketCloseInfo> {
    if (this.#closedInfo) {
      return this.#closedInfo;
    }
    this.#readyState = READY_STATE_CLOSING;
    this.#drainingClose = true;
    if (!this.#handleReleased) {
      this.#handle.call("close", code ?? null, reason);
    }
    void this.#ensureDrainUntilClose();
    return this.closed;
  }

  async #ensureDrainUntilClose(): Promise<void> {
    if (this.#inFlightRead) {
      await this.#inFlightRead.catch(() => null);
    }
    while (!this.#closedInfo && this.#drainingClose) {
      const ev = await this.nextEvent();
      if (!ev) break;
    }
  }

  dispose(): void {
    if (this.#handleReleased) return;
    this.#handleReleased = true;
    try {
      this.#handle.close();
    } catch {
      // Resource already closed.
    }
  }

  #finishClosed(info: WebSocketCloseInfo): void {
    if (this.#closedInfo) return;
    this.#readyState = READY_STATE_CLOSED;
    this.#closedInfo = info;
    this.dispose();
    this.#closedResolve(info);
  }

  async waitOpen(signal?: AbortSignal): Promise<this> {
    const onAbort = () => {
      void this.#reader.cancel(abortError(signal?.reason)).catch(() => {});
      this.dispose();
    };
    signal?.addEventListener("abort", onAbort, { once: true });

    try {
      for (;;) {
        const step = await this.#reader.read();
        if (step.done) {
          const info = { code: 1006, reason: "", wasClean: false };
          this.#finishClosed(info);
          throw new Error("WebSocket closed before opening");
        }
        const ev = step.value;
        if (ev.type === "open") {
          this.#protocol = ev.protocol;
          this.#readyState = READY_STATE_OPEN;
          return this;
        }
        if (ev.type === "error") {
          throw new Error(ev.message);
        }
        if (ev.type === "close") {
          const info = { code: ev.code, reason: ev.reason, wasClean: ev.clean };
          this.#finishClosed(info);
          throw new Error(ev.reason || `WebSocket closed (${ev.code})`);
        }
      }
    } catch (err) {
      if (signal?.aborted) {
        this.dispose();
        throw abortError(signal.reason);
      }
      this.#finishClosed({ code: 1006, reason: "", wasClean: false });
      throw err;
    } finally {
      signal?.removeEventListener("abort", onAbort);
    }
  }

  async nextEvent(): Promise<NativeWebSocketEvent | null> {
    if (this.#inFlightRead) {
      return this.#inFlightRead;
    }
    const readPromise = (async () => {
      try {
        const step = await this.#reader.read();
        if (step.done) {
          this.#finishClosed({ code: 1006, reason: "", wasClean: false });
          return null;
        }
        const ev = step.value;
        if (ev.type === "open") {
          this.#protocol = ev.protocol;
          this.#readyState = READY_STATE_OPEN;
        } else if (ev.type === "close") {
          this.#finishClosed({ code: ev.code, reason: ev.reason, wasClean: ev.clean });
        }
        return ev;
      } finally {
        this.#inFlightRead = null;
      }
    })();
    this.#inFlightRead = readPromise;
    return readPromise;
  }

  async *messages(): AsyncIterableIterator<WebSocketMessage> {
    for (;;) {
      const ev = await this.nextEvent();
      if (!ev) return;
      if (ev.type === "message") {
        yield ev.text ? ev.data : toUint8Array(ev.data);
      } else if (ev.type === "close") {
        return;
      } else if (ev.type === "error") {
        throw new Error(ev.message);
      }
    }
  }
}

export function openClient(
  transport: WebSocketTransport,
  url: string,
  options: ConnectOptions = {},
): WebSocketClient {
  const protocols = normalizeProtocols(options.protocols);
  const handle = transport.open(url, protocols);
  try {
    const id = handle.call("id");
    const stream = transport.events(id, options.signal);
    return new WebSocketClient(url, handle, stream);
  } catch (err) {
    try {
      handle.close();
    } catch {
      // ignore
    }
    throw err;
  }
}

export async function connectWith(
  transport: WebSocketTransport,
  url: string,
  options: ConnectOptions = {},
): Promise<WebSocketClient> {
  if (options.signal?.aborted) {
    throw abortError(options.signal.reason);
  }
  const client = openClient(transport, url, options);
  return client.waitOpen(options.signal);
}
