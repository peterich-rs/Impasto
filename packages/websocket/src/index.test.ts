import { assert, test } from "impasto:test";
import {
  connectWith,
  READY_STATE_CLOSED,
  READY_STATE_OPEN,
  type WebSocketTransport,
} from "./client.ts";
import { createWebSocketClass } from "./compat-impl.ts";
import type { NativeWebSocketEvent, NativeWebSocketHandle } from "@impasto/websocket/native";

interface HarnessHandle {
  sentText: string[];
  sentBinary: Uint8Array[];
  closedArgs: Array<{ code: number | null; reason?: string }>;
  disposed: boolean;
  pushEvent(event: NativeWebSocketEvent): void;
  end(): void;
}

function mockWebSocketTransport(): {
  transport: WebSocketTransport;
  lastHandle: () => HarnessHandle;
} {
  let latest: HarnessHandle | null = null;
  const transport: WebSocketTransport = {
    open(_url, _protocols) {
      let controllerRef: ReadableStreamDefaultController<NativeWebSocketEvent> | null = null;
      const queued: NativeWebSocketEvent[] = [];
      let ended = false;
      let readyState = 0;
      const protocol = "chat";
      let closedFlag = false;

      const state: HarnessHandle = {
        sentText: [],
        sentBinary: [],
        closedArgs: [],
        disposed: false,
        pushEvent(event) {
          if (event.type === "open") readyState = 1;
          else if (event.type === "close") readyState = 3;
          if (controllerRef) controllerRef.enqueue(event);
          else queued.push(event);
        },
        end() {
          ended = true;
          controllerRef?.close();
        },
      };
      latest = state;

      const handle: NativeWebSocketHandle & { streamController?: ReadableStreamDefaultController<NativeWebSocketEvent> } = {
        id: () => 1,
        readyState: () => readyState,
        protocol: () => readyState >= 1 ? protocol : "",
        bufferedAmount: () => 0,
        sendText(text) {
          state.sentText.push(text);
          return 0;
        },
        sendBinary(data) {
          state.sentBinary.push(data instanceof Uint8Array ? data : new Uint8Array(data));
          return 0;
        },
        closeSocket(code, reason) {
          readyState = 2;
          state.closedArgs.push({ code: code ?? null, reason: reason ?? undefined });
          state.pushEvent({ type: "close", code: code ?? 1000, reason: reason ?? "", clean: true });
        },
        close() {
          if (!closedFlag) {
            closedFlag = true;
            state.disposed = true;
          }
        },
      };

      const originalTransportEvents = new ReadableStream<NativeWebSocketEvent>({
        start(controller) {
          controllerRef = controller;
          for (const ev of queued.splice(0)) controller.enqueue(ev);
          if (ended) controller.close();
        },
      });
      streamById.set(1, originalTransportEvents);
      return handle;
    },
    events(id) {
      const stream = streamById.get(id);
      if (!stream) throw new Error("missing stream");
      streamById.delete(id);
      return stream;
    },
  };
  const streamById = new Map<number, ReadableStream<NativeWebSocketEvent>>();
  return {
    transport,
    lastHandle: () => {
      if (!latest) throw new Error("no handle opened");
      return latest;
    },
  };
}

test("connectWith waits for open, iterates messages, and closes cleanly", async () => {
  const { transport, lastHandle } = mockWebSocketTransport();
  const pending = connectWith(transport, "ws://example.test/chat", { protocols: ["chat"] });
  const h = lastHandle();
  h.pushEvent({ type: "open", protocol: "chat" });

  const ws = await pending;
  assert.equal(ws.readyState, READY_STATE_OPEN);
  assert.equal(ws.protocol, "chat");

  ws.send("ping");
  ws.send(new Uint8Array([9, 8]));
  assert.deepEqual(h.sentText, ["ping"]);
  assert.deepEqual(h.sentBinary, [new Uint8Array([9, 8])]);

  h.pushEvent({ type: "message", text: true, data: "pong" });
  h.pushEvent({ type: "message", text: false, data: new Uint8Array([1, 2]).buffer });

  const received: Array<string | Uint8Array> = [];
  const iterator = ws.messages();
  received.push((await iterator.next()).value);
  received.push((await iterator.next()).value);
  assert.equal(received[0], "pong");
  assert.deepEqual(received[1], new Uint8Array([1, 2]));

  const closed = await ws.close(1000, "bye");
  assert.equal(closed.wasClean, true);
  assert.equal(closed.code, 1000);
  assert.equal(ws.readyState, READY_STATE_CLOSED);
  assert.equal(h.disposed, true);
});

test("WHATWG WebSocket compat class validates inputs and dispatches lifecycle events", async () => {
  const { transport, lastHandle } = mockWebSocketTransport();
  const WebSocket = createWebSocketClass(transport);

  assert.throws(() => new WebSocket("https://example.test"), SyntaxError);

  const socket = new WebSocket("ws://example.test/stream");
  assert.throws(() => socket.send("too early"), (err: unknown) => err instanceof Error && err.name === "InvalidStateError");
  assert.throws(() => socket.close(1006), Error);

  const seen: string[] = [];
  await new Promise<void>((resolve) => {
    socket.addEventListener("open", () => {
      seen.push("open");
      socket.send("hello");
    });
    socket.onmessage = (event) => {
      seen.push(`msg:${String(event.data)}`);
      socket.close(1000, "done");
    };
    socket.onclose = (event) => {
      seen.push(`close:${event.code}:${event.wasClean}`);
      resolve();
    };
    const h = lastHandle();
    h.pushEvent({ type: "open", protocol: "chat" });
    h.pushEvent({ type: "message", text: true, data: "world" });
  });

  assert.deepEqual(seen, ["open", "msg:world", "close:1000:true"]);
});
