import { advanceFrames, assert, test } from "impasto:test";
import { createClient, createLatest, HttpError, type HttpTransport } from "./client.ts";
import { createFetch, Request, Response } from "./fetch-impl.ts";
import { FormData } from "./form-data.ts";
import { Headers } from "./headers.ts";
import type { NativeHttpEvent, NativeHttpRequest } from "@impasto/http/native";

function mockTransport(
  handler: (
    request: NativeHttpRequest,
    signal?: AbortSignal,
  ) => AsyncIterable<NativeHttpEvent> | Iterable<NativeHttpEvent>,
): HttpTransport {
  return {
    send(request, signal) {
      const iter = handler(request, signal);
      const iterator =
        Symbol.asyncIterator in iter
          ? iter[Symbol.asyncIterator]()
          : (function* () {
              yield* iter;
            })();
      return new ReadableStream<NativeHttpEvent>({
        async pull(controller) {
          await advanceFrames();
          if (signal?.aborted) {
            controller.error(signal.reason ?? Object.assign(new Error("aborted"), { name: "AbortError" }));
            return;
          }
          const next = await iterator.next();
          if (next.done) {
            controller.close();
          } else {
            controller.enqueue(next.value);
          }
        },
        async cancel(reason) {
          await iterator.return?.(reason);
        },
      }, { highWaterMark: 0 });
    },
  };
}

test("Headers normalizes names, joins values, and preserves Set-Cookie", () => {
  const headers = new Headers([
    ["X-Test", " one "],
    ["x-test", "two"],
    ["Set-Cookie", "a=1; Path=/"],
    ["set-cookie", "b=2; HttpOnly"],
  ]);
  assert.equal(headers.get("X-TEST"), "one, two");
  assert.deepEqual(headers.getSetCookie(), ["a=1; Path=/", "b=2; HttpOnly"]);
  assert.deepEqual([...headers.entries()], [
    ["set-cookie", "a=1; Path=/"],
    ["set-cookie", "b=2; HttpOnly"],
    ["x-test", "one, two"],
  ]);
  assert.throws(() => new Headers({ "bad header": "1" }), TypeError);
});

test("FormData serializes strings and binary parts", async () => {
  const form = new FormData();
  form.append("title", "hello");
  form.append("file", new Uint8Array([1, 2, 3]), "data.bin");
  const { contentType, body } = await form.serializeMultipart();
  assert.match(contentType, /^multipart\/form-data; boundary=----impastoFormBoundary/);
  const text = new TextDecoder().decode(body);
  assert.match(text, /name="title"\r\n\r\nhello/);
  assert.match(text, /name="file"; filename="data\.bin"/);
});

test("HttpClient normalizes ArrayBuffer chunks and throws HttpError on 4xx/5xx", async () => {
  let seenRequest: NativeHttpRequest | undefined;
  const transport = mockTransport((req) => {
    seenRequest = req;
    const url = new URL(req.url);
    if (url.pathname === "/err") {
      return [
        { type: "head", status: 404, url: req.url, redirected: false, headers: [] },
        { type: "chunk", data: new TextEncoder().encode("missing").buffer },
      ];
    }
    return [
      {
        type: "head",
        status: 201,
        url: req.url,
        redirected: false,
        headers: [["Content-Type", "application/json"]],
      },
      { type: "chunk", data: new TextEncoder().encode('{"ok":').buffer },
      { type: "chunk", data: new TextEncoder().encode("true}").buffer },
    ];
  });

  const client = createClient(transport, {
    baseUrl: "https://api.example.test",
    headers: { "X-App": "impasto" },
  });

  const res = await client.post("/items", {
    query: { page: 2, tag: null },
    json: { name: "box" },
  });
  await advanceFrames();
  assert.equal(res.status, 201);
  assert.equal(res.ok, true);
  assert.deepEqual(await res.json(), { ok: true });
  await advanceFrames();
  assert.equal(seenRequest?.url, "https://api.example.test/items?page=2");
  assert.equal(seenRequest?.method, "POST");
  assert.deepEqual(JSON.parse(typeof seenRequest?.body === "string" ? seenRequest.body : new TextDecoder().decode(seenRequest?.body ?? undefined)), { name: "box" });

  await assert.rejects(
    () => client.get("/err"),
    (err: unknown) => err instanceof HttpError && err.code === "http-status" && err.status === 404,
  );
});

test("pull-driven ReadableStream applies backpressure until consumed", async () => {
  let pulled = 0;
  const transport = mockTransport(async function* (req) {
    yield { type: "head", status: 200, url: req.url, redirected: false, headers: [] };
    for (let i = 1; i <= 3; i += 1) {
      pulled += 1;
      yield { type: "chunk", data: new Uint8Array([i]) };
    }
  });

  const client = createClient(transport);
  const res = await client.get("https://example.test/stream");
  assert.equal(pulled, 0);
  const reader = res.body!.getReader();
  const first = await reader.read();
  assert.deepEqual(first.value, new Uint8Array([1]));
  assert.equal(pulled, 1);
  await reader.cancel();
});

test("createFetch maps wire errors, handles Request/Response/clone, and resolves 404", async () => {
  const transport = mockTransport((req) => {
    if (req.url.endsWith("/timeout")) {
      throw new Error("[impasto:http:timeout] TimeoutError");
    }
    if (req.url.endsWith("/network")) {
      throw new Error("[impasto:http:network] connection reset");
    }
    if (req.url.endsWith("/404")) {
      return [
        { type: "head", status: 404, url: req.url, redirected: false, headers: [] },
        { type: "chunk", data: new TextEncoder().encode("nope") },
      ];
    }
    return [
      {
        type: "head",
        status: 200,
        url: "https://example.test/final",
        redirected: true,
        headers: [["Content-Type", "application/json"]],
      },
      { type: "chunk", data: new TextEncoder().encode('{"v":1}') },
    ];
  });

  const fetchImpl = createFetch(transport);
  const r1 = await fetchImpl(new Request("https://example.test/start"));
  assert.equal(r1.redirected, true);
  assert.equal(r1.url, "https://example.test/final");
  const r2 = r1.clone();
  assert.deepEqual(await r1.json(), { v: 1 });
  assert.equal(await r2.text(), '{"v":1}');
  await advanceFrames();

  const r404 = await fetchImpl("https://example.test/404");
  assert.equal(r404.status, 404);
  assert.equal(r404.ok, false);
  assert.equal(await r404.text(), "nope");
  await advanceFrames();

  await assert.rejects(
    () => fetchImpl("https://example.test/timeout"),
    (err: unknown) => err instanceof Error && err.name === "TimeoutError",
  );
  await assert.rejects(
    () => fetchImpl("https://example.test/network"),
    (err: unknown) => err instanceof TypeError,
  );
});

test("createLatest aborts the previous request when a newer one starts", async () => {
  const seen: AbortSignal[] = [];
  const latest = createLatest(async (signal, label: string) => {
    seen.push(signal);
    return label;
  });
  await latest.run("first");
  assert.equal(seen[0].aborted, false);
  await latest.run("second");
  assert.equal(seen[0].aborted, true);
  assert.equal(seen[1].aborted, false);
  latest.abort();
  assert.equal(seen[1].aborted, true);
});

test("HTTP retry replays idempotent bytes, runs interceptors once and reports progress", async () => {
  let attempts = 0;
  let intercepted = 0;
  let lastLoaded = 0;
  const client = createClient(mockTransport(function* (request) {
    attempts++;
    assert.deepEqual(request.headers, [["x-session", "active"]]);
    yield { type: "head", url: request.url, status: attempts === 1 ? 503 : 200, redirected: false, headers: [] };
    if (attempts === 2) {
      yield { type: "downloadProgress", loaded: 2, total: 2 };
      yield { type: "chunk", data: new Uint8Array([111, 107]) };
    }
  }), { retry: { maxRetries: 1, baseDelayMs: 0, jitter: false } });
  client.interceptors.request.use(request => {
    intercepted++;
    return { ...request, headers: { "X-Session": "active" } };
  });
  const result = await client.send("https://example.test", {
    onDownloadProgress: progress => { lastLoaded = progress.loaded; },
  });
  assert.equal(await result.text(), "ok");
  assert.equal(attempts, 2);
  assert.equal(intercepted, 1);
  assert.equal(lastLoaded, 2);
});

test("scripted HTTP download streams progress and commits the headless file", async () => {
  const { mockHttp } = await import("./testing.ts");
  const filesystem = await import("@impasto/file-system");
  const mock = mockHttp([{ status: 429, headers: { "Retry-After": "0" } }, { chunks: ["hel", "lo"] }],
    { retry: { maxRetries: 1, baseDelayMs: 0, jitter: false } });
  const destinationPath = "http-mock-test.txt";
  try {
    let loaded = 0;
    const result = await mock.client.downloadToFile("https://example.test/file", {
      destinationPath,
      onDownloadProgress: event => { loaded = event.loaded; },
    });
    assert.equal(result.bytesWritten, 5);
    assert.equal(loaded, 5);
    assert.equal(await filesystem.readText(destinationPath, "cache"), "hello");
    assert.equal(mock.requests.length, 2);
  } finally { await filesystem.delete(destinationPath, { scope: "cache" }).catch(() => undefined); }
});
