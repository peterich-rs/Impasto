import {
  openClient,
  READY_STATE_CLOSED,
  READY_STATE_CLOSING,
  READY_STATE_CONNECTING,
  READY_STATE_OPEN,
  WebSocketClient,
  type WebSocketTransport,
} from "./client.ts";

export class WebSocketMessageEvent extends Event {
  readonly data: string | ArrayBuffer | unknown;
  readonly origin: string;

  constructor(type: string, init: { data: string | ArrayBuffer | unknown; origin?: string }) {
    super(type);
    this.data = init.data;
    this.origin = init.origin ?? "";
  }
}

export class WebSocketCloseEvent extends Event {
  readonly code: number;
  readonly reason: string;
  readonly wasClean: boolean;

  constructor(
    type: string,
    init: { code?: number; reason?: string; wasClean?: boolean } = {},
  ) {
    super(type);
    this.code = init.code ?? 1005;
    this.reason = init.reason ?? "";
    this.wasClean = init.wasClean ?? false;
  }
}

function invalidStateError(message: string): Error {
  if (typeof DOMException === "function") {
    return new DOMException(message, "InvalidStateError");
  }
  return Object.assign(new Error(message), { name: "InvalidStateError" });
}

function invalidAccessError(message: string): Error {
  if (typeof DOMException === "function") {
    return new DOMException(message, "InvalidAccessError");
  }
  return Object.assign(new Error(message), { name: "InvalidAccessError" });
}

const utf8Encoder = new TextEncoder();

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

export function createWebSocketClass(transport: WebSocketTransport) {
  return class WebSocket extends EventTarget {
    static readonly CONNECTING = READY_STATE_CONNECTING;
    static readonly OPEN = READY_STATE_OPEN;
    static readonly CLOSING = READY_STATE_CLOSING;
    static readonly CLOSED = READY_STATE_CLOSED;

    readonly CONNECTING = READY_STATE_CONNECTING;
    readonly OPEN = READY_STATE_OPEN;
    readonly CLOSING = READY_STATE_CLOSING;
    readonly CLOSED = READY_STATE_CLOSED;

    readonly url: string;
    readonly extensions = "";
    binaryType: "arraybuffer" | "blob" = "arraybuffer";

    onopen: ((event: Event) => void) | null = null;
    onmessage: ((event: WebSocketMessageEvent) => void) | null = null;
    onerror: ((event: Event) => void) | null = null;
    onclose: ((event: WebSocketCloseEvent) => void) | null = null;

    readonly #client: WebSocketClient;
    readonly #origin: string;
    #state = READY_STATE_CONNECTING;

    constructor(url: string | URL, protocols?: string | string[]) {
      super();
      const rawUrl = url instanceof URL ? url.toString() : String(url);
      let parsed: URL;
      try {
        parsed = new URL(rawUrl);
      } catch {
        throw new SyntaxError(`Invalid WebSocket URL: ${rawUrl}`);
      }
      if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
        throw new SyntaxError(`Unsupported WebSocket scheme: ${parsed.protocol}`);
      }
      this.url = parsed.toString();
      this.#origin = parsed.origin;
      this.#client = openClient(transport, this.url, { protocols });
      void this.#pump();
    }

    get readyState(): number {
      return this.#state;
    }

    get protocol(): string {
      return this.#client.protocol;
    }

    get bufferedAmount(): number {
      return this.#client.bufferedAmount;
    }

    send(data: string | ArrayBuffer | ArrayBufferView): void {
      if (this.#state === READY_STATE_CONNECTING) {
        throw invalidStateError("WebSocket is still in CONNECTING state.");
      }
      if (this.#state !== READY_STATE_OPEN) {
        return;
      }
      this.#client.send(data);
    }

    close(code?: number, reason?: string): void {
      if (code !== undefined) {
        if (!Number.isInteger(code) || (code !== 1000 && (code < 3000 || code > 4999))) {
          throw invalidAccessError(`Invalid WebSocket close code: ${code}`);
        }
      }
      if (reason !== undefined && utf8Encoder.encode(reason).byteLength > 123) {
        throw new SyntaxError("WebSocket close reason exceeds 123 UTF-8 bytes.");
      }
      if (this.#state === READY_STATE_CLOSING || this.#state === READY_STATE_CLOSED) {
        return;
      }
      this.#state = READY_STATE_CLOSING;
      void this.#client.close(code, reason).catch(() => {});
    }

    #emit(event: Event): void {
      if (event.type === "open") this.onopen?.call(this, event);
      else if (event.type === "message") this.onmessage?.call(this, event as WebSocketMessageEvent);
      else if (event.type === "error") this.onerror?.call(this, event);
      else if (event.type === "close") this.onclose?.call(this, event as WebSocketCloseEvent);
      this.dispatchEvent(event);
    }

    async #pump(): Promise<void> {
      try {
        for (;;) {
          const ev = await this.#client.nextEvent();
          if (!ev) {
            if (this.#state !== READY_STATE_CLOSED) {
              this.#state = READY_STATE_CLOSED;
              this.#emit(
                new WebSocketCloseEvent("close", { code: 1006, reason: "", wasClean: false }),
              );
            }
            return;
          }
          if (ev.type === "open") {
            if (this.#state === READY_STATE_CONNECTING) {
              this.#state = READY_STATE_OPEN;
              this.#emit(new Event("open"));
            }
          } else if (ev.type === "message") {
            let payload: string | ArrayBuffer | unknown;
            if (typeof ev.data === "string") {
              payload = ev.data;
            } else {
              const bytes =
                ev.data instanceof Uint8Array ? ev.data : new Uint8Array(ev.data);
              if (this.binaryType === "blob" && typeof Blob === "function") {
                payload = new Blob([toArrayBuffer(bytes)]);
              } else {
                payload = toArrayBuffer(bytes);
              }
            }
            this.#emit(
              new WebSocketMessageEvent("message", { data: payload, origin: this.#origin }),
            );
          } else if (ev.type === "error") {
            this.#emit(new Event("error"));
          } else if (ev.type === "close") {
            this.#state = READY_STATE_CLOSED;
            this.#emit(
              new WebSocketCloseEvent("close", {
                code: ev.code,
                reason: ev.reason,
                wasClean: ev.clean,
              }),
            );
            return;
          }
        }
      } catch {
        this.#client.dispose();
        if (this.#state !== READY_STATE_CLOSED) {
          this.#state = READY_STATE_CLOSED;
          this.#emit(new Event("error"));
          this.#emit(
            new WebSocketCloseEvent("close", { code: 1006, reason: "", wasClean: false }),
          );
        }
      }
    }
  };
}
