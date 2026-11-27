import type { ClipboardFormat } from "./types";

export interface MemoryContent {
  readonly text?: string | null;
  readonly html?: string | null;
  readonly uri?: string | null;
  readonly imagePng?: Uint8Array | ArrayBuffer | null;
}

export function memoryClipboard(initial: MemoryContent = {}) {
  let content = copy(initial);
  const subscribers = new Set<ReadableStreamDefaultController<ClipboardFormat[]>>();

  function copy(value: MemoryContent) {
    return {
      text: value.text ?? null,
      html: value.html ?? null,
      uri: value.uri ?? null,
      imagePng: value.imagePng == null ? null
        : value.imagePng instanceof ArrayBuffer ? new Uint8Array(value.imagePng.slice(0))
        : new Uint8Array(value.imagePng),
    };
  }

  function formats(): ClipboardFormat[] {
    const result: ClipboardFormat[] = [];
    if (content.text !== null) result.push("text");
    if (content.html !== null) result.push("html");
    if (content.uri !== null) result.push("uri");
    if (content.imagePng !== null) result.push("image");
    return result;
  }

  function replace(value: MemoryContent) {
    content = copy(value);
    for (const controller of subscribers) controller.enqueue(formats());
  }

  return {
    getString: () => content.text ?? "",
    setString: (text: string) => replace({ text }),
    hasStrings: async () => content.text !== null,
    hasImages: async () => content.imagePng !== null,
    hasHtml: async () => content.html !== null,
    availableFormats: async () => formats(),
    read: async () => copy(content),
    async write(value: MemoryContent) {
      if (Object.values(value).every(field => field == null)) {
        throw Object.assign(new Error("Clipboard content needs at least one format"), {
          scope: "clipboard", code: "empty-content",
        });
      }
      replace(value);
    },
    clear: async () => replace({}),
    watchChanges(signal?: AbortSignal) {
      let stop = () => {};
      return new ReadableStream<ClipboardFormat[]>({
        start(controller) {
          if (signal?.aborted) { controller.error(signal.reason); return; }
          const abort = () => { stop(); controller.error(signal?.reason); };
          stop = () => { subscribers.delete(controller); signal?.removeEventListener("abort", abort); };
          signal?.addEventListener("abort", abort, { once: true });
          subscribers.add(controller);
        },
        cancel() { stop(); },
      });
    },
    simulateExternalCopy: replace,
  };
}
