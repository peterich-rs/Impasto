import { installMock } from "./driver";
import type { ShareFileInput, ShareOptions, ShareResult } from "./types";

export function mockShare(initial: ShareResult = { status: "success" }) {
  let result = initial;
  const calls: ShareOptions[] = [];
  const restore = installMock({
    async share(options) {
      calls.push({
        text: options.text ?? undefined, title: options.title ?? undefined, url: options.url ?? undefined,
        subject: options.subject ?? undefined, anchor: options.anchor ?? undefined,
        files: options.files?.flatMap((file): ShareFileInput[] => {
          if ("path" in file && file.path) {
            return [{ path: file.path, name: file.name ?? undefined, mimeType: file.mimeType ?? undefined,
              scope: file.scope ?? undefined }];
          }
          if ("bytes" in file && file.bytes && file.name) {
            return [{ name: file.name, bytes: new Uint8Array(file.bytes), mimeType: file.mimeType ?? undefined }];
          }
          return [];
        }),
      });
      return { ...result };
    },
  });
  return { calls: calls as readonly ShareOptions[], respondWith(value: ShareResult) { result = value; }, restore };
}
