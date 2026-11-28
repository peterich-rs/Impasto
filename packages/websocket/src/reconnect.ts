import type { ConnectOptions, WebSocketClient, WebSocketMessage } from "./client.ts";

export type ReconnectStatus = "connecting" | "open" | "reconnecting" | "closed";
export interface ReconnectPolicy {
  readonly maxRetries?: number;
  readonly baseDelayMs?: number;
  readonly maxDelayMs?: number;
  readonly jitter?: boolean;
  readonly stopOnCloseCodes?: readonly number[];
  readonly reconnectOnOnline?: boolean;
}

export class ReconnectingWebSocket {
  readonly #stop = new AbortController();
  readonly #states = new Set<ReadableStreamDefaultController<ReconnectStatus>>();
  readonly #messages = new Set<ReadableStreamDefaultController<WebSocketMessage>>();
  #status: ReconnectStatus = "connecting";
  #current?: WebSocketClient;
  #wake?: () => void;
  #closing = false;
  #networkRestored = false;

  constructor(
    connect: (url: string, options?: ConnectOptions) => Promise<WebSocketClient>,
    url: string, options: ConnectOptions & { readonly reconnect?: ReconnectPolicy } = {},
    observeOnline?: (signal: AbortSignal) => ReadableStream<{ isConnected: boolean }>,
  ) {
    const { reconnect = {}, signal } = options;
    const stop = () => this.#stop.abort(signal?.reason);
    if (signal?.aborted) stop();
    else signal?.addEventListener("abort", stop, { once: true });
    const online = () => {
      if (!this.#wake) return;
      this.#networkRestored = true;
      this.#wake();
    };
    if (reconnect.reconnectOnOnline) {
      globalThis.addEventListener?.("online", online);
      if (observeOnline) {
        void this.#observeOnline(observeOnline, online).catch(() => undefined);
      }
    }
    void this.#run(connect, url, { ...options, signal: this.#stop.signal }, reconnect).finally(() => {
      this.#publish("closed");
      this.#states.forEach(controller => controller.close());
      this.#messages.forEach(controller => controller.close());
      this.#states.clear();
      this.#messages.clear();
      signal?.removeEventListener("abort", stop);
      globalThis.removeEventListener?.("online", online);
      this.#stop.abort();
    });
  }

  get status(): ReconnectStatus { return this.#status; }

  watchStatus(): ReadableStream<ReconnectStatus> {
    return this.#watch(this.#states, () => this.#status);
  }

  async *messages(): AsyncIterableIterator<WebSocketMessage> {
    const reader = this.#watch(this.#messages).getReader();
    try {
      for (;;) {
        const item = await reader.read();
        if (item.done) return;
        yield item.value;
      }
    } finally { await reader.cancel(); reader.releaseLock(); }
  }

  send(data: string | ArrayBuffer | ArrayBufferView): number {
    if (!this.#current || this.#status !== "open") throw new Error("[websocket:closed] WebSocket is disconnected");
    return this.#current.send(data);
  }

  async close(code?: number, reason?: string): Promise<void> {
    this.#closing = true;
    try { await this.#current?.close(code, reason); }
    finally { this.#stop.abort(); }
  }

  async #observeOnline(
    observe: (signal: AbortSignal) => ReadableStream<{ isConnected: boolean }>,
    online: () => void,
  ) {
    const reader = observe(this.#stop.signal).getReader();
    try {
      let connected: boolean | undefined;
      while (!this.#stop.signal.aborted) {
        const result = await reader.read();
        if (result.done) break;
        if (connected === false && result.value.isConnected) online();
        connected = result.value.isConnected;
      }
    } finally { reader.releaseLock(); }
  }

  #watch<T>(subscribers: Set<ReadableStreamDefaultController<T>>, initial?: () => T): ReadableStream<T> {
    let entry: ReadableStreamDefaultController<T> | undefined;
    return new ReadableStream<T>({
      start: controller => {
        if (initial) controller.enqueue(initial());
        if (this.#status === "closed") controller.close();
        else { entry = controller; subscribers.add(controller); }
      },
      cancel: () => { if (entry) subscribers.delete(entry); },
    }, { highWaterMark: 16 });
  }

  #publish(state: ReconnectStatus): void {
    if (this.#status === state) return;
    this.#status = state;
    this.#states.forEach(controller => controller.enqueue(state));
  }

  async #run(
    connect: (url: string, options?: ConnectOptions) => Promise<WebSocketClient>,
    url: string, options: ConnectOptions, policy: ReconnectPolicy,
  ) {
    let retries = 0;
    const stopCodes = policy.stopOnCloseCodes ?? [1000, 1001];
    while (!this.#stop.signal.aborted && !this.#closing) {
      try {
        const socket = await connect(url, options);
        this.#current = socket;
        retries = 0;
        this.#publish("open");
        for await (const message of socket.messages()) {
          for (const controller of this.#messages) {
            if ((controller.desiredSize ?? 0) > 0) controller.enqueue(message);
            else {
              controller.error(new Error("WebSocket message consumer is too slow"));
              this.#messages.delete(controller);
            }
          }
        }
        if (stopCodes.includes((await socket.closed).code)) return;
      } catch {
        // Transport errors also use the reconnect policy.
      } finally {
        this.#current = undefined;
      }

      if (this.#stop.signal.aborted || retries >= (policy.maxRetries ?? Infinity)) return;
      if (this.#closing) return;
      this.#publish("reconnecting");
      const ceiling = Math.max(0, Math.min(policy.maxDelayMs ?? 30_000,
        (policy.baseDelayMs ?? 500) * (2 ** Math.min(30, retries++))));
      await this.#delay((policy.jitter ?? true) ? Math.random() * ceiling : ceiling);
      if (this.#networkRestored) {
        this.#networkRestored = false;
        retries = 0;
      }
    }
  }

  async #delay(duration: number): Promise<void> {
    await new Promise<void>(resolve => {
      const stop = () => {
        clearTimeout(timer);
        this.#stop.signal.removeEventListener("abort", stop);
        this.#wake = undefined;
        resolve();
      };
      const timer = setTimeout(stop, duration);
      this.#wake = stop;
      this.#stop.signal.addEventListener("abort", stop, { once: true });
      if (this.#stop.signal.aborted) stop();
    });
  }
}
