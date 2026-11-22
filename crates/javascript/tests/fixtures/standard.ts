const base = new URL("https://example.test/");
base.searchParams.append("filter", "yellow");
const target = new URL("items", base);
const controller = new AbortController();
let abortedFromEvent = false;
controller.signal.addEventListener("abort", () => {
  abortedFromEvent = true;
});
const bytes: Uint8Array = new TextEncoder().encode("A");
const text: string = new TextDecoder("utf8").decode(bytes);
const encoding: "utf-8" = new TextEncoder().encoding;
const [[filter, value]] = base.searchParams;
if (bytes[0] !== 65 || text !== "A" || encoding !== "utf-8"
    || !Object.getOwnPropertyDescriptor(TextEncoder.prototype, "encoding")?.get
    || performance.now() !== 0
    || target.href !== "https://example.test/items"
    || filter !== "filter" || value !== "yellow") {
  throw new Error("standard library fixture returned unexpected values");
}
controller.abort("fixture");
if (!controller.signal.aborted || controller.signal.reason !== "fixture" || !abortedFromEvent) {
  throw new Error("abort signal did not update or fire listeners");
}

const stream = new ReadableStream<Uint8Array>({
  start(ctrl) {
    ctrl.enqueue(bytes);
    ctrl.close();
  },
});

async function checkTypes(readable: ReadableStream<Uint8Array>) {
  const reader = readable.getReader();
  const chunk = await reader.read();
  if (!chunk.done) console.info(new TextDecoder().decode(chunk.value));
  reader.releaseLock();
  setTimeout((message: string) => console.log(message), 0, "ready");

  // @ts-expect-error The published decoder accepts UTF-8 labels.
  new TextDecoder("windows-1252");
  // @ts-expect-error The supported byte streams have default readers.
  readable.getReader({ mode: "byob" });
  // @ts-expect-error The runtime has no DOM.
  document.createElement("div");
}

const streamCheck = checkTypes(stream);
void streamCheck;

const clonedGraph: { bytes: Uint8Array; self?: unknown } = { bytes: new Uint8Array([1, 2, 3]) };
clonedGraph.self = clonedGraph;
const cloned = structuredClone(clonedGraph);
if (cloned.self !== cloned || cloned.bytes === clonedGraph.bytes || cloned.bytes[2] !== 3) {
  throw new Error("structured clone must preserve cycles and copy bytes");
}
if (atob(btoa("\u00ff\u0000")) !== "\u00ff\u0000") throw new Error("Base64 binary string");
if (new DOMException("stop", "AbortError").code !== DOMException.ABORT_ERR) throw new Error("DOMException");
const random = new Uint16Array(2);
if (crypto.getRandomValues(random) !== random || !/^[0-9a-f-]{36}$/.test(crypto.randomUUID())) {
  throw new Error("crypto random values and UUID");
}
queueMicrotask(() => {});
