import { installMock } from "./driver";
import type { PickedMedia } from "./types";

export function mockImagePicker(initial: readonly PickedMedia[] = []) {
  let files = [...initial];
  let camera = true;
  function capture(kind: "image" | "video"): PickedMedia | null {
    if (!camera) throw new Error("[image-picker:camera-unavailable] Camera is unavailable");
    return files.find(item => item.kind === kind) ?? null;
  }
  const restore = installMock({
    async pickMedia(options) {
      const kind = options?.mediaType ?? "image";
      return files.filter(file => kind === "any" || file.kind === kind).slice(0, options?.multiple ? options.maxSelection ?? files.length : 1);
    },
    async capturePhoto() { return capture("image"); },
    async captureVideo() { return capture("video"); },
    async cleanupCache() { files = []; },
  });
  return { setFiles(items: readonly PickedMedia[]) { files = [...items]; }, setCameraAvailable(value: boolean) { camera = value; }, restore };
}
