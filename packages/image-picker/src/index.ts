import { driver } from "./driver";
import type { CapturePhotoOptions, CaptureVideoOptions, PickMediaOptions, PickedMedia } from "./types";
export type * from "./types";
export function pickMedia(options?: PickMediaOptions): Promise<PickedMedia[]> { return driver().pickMedia(options); }
export async function pickImage(options?: Omit<PickMediaOptions, "mediaType" | "multiple">): Promise<PickedMedia | null> {
  return (await pickMedia({ ...options, mediaType: "image", multiple: false }))[0] ?? null;
}
export async function pickVideo(options?: Omit<PickMediaOptions, "mediaType" | "multiple" | "process">): Promise<PickedMedia | null> {
  return (await pickMedia({ ...options, mediaType: "video", multiple: false }))[0] ?? null;
}
export async function capturePhoto(options?: CapturePhotoOptions): Promise<PickedMedia | null> {
  return await driver().capturePhoto(options) ?? null;
}
export async function captureVideo(options?: CaptureVideoOptions): Promise<PickedMedia | null> {
  return await driver().captureVideo(options) ?? null;
}
export const cleanupCache = () => driver().cleanupCache();
