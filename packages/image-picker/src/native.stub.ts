import type { CapturePhotoOptions, CaptureVideoOptions, PickMediaOptions, PickedMedia } from "./types";
export const isImagePickerHostAvailable = () => false;
function unavailable(): never { throw new Error("[image-picker:host-unavailable] Media selection requires a native host"); }
export async function pickMedia(_options?: PickMediaOptions): Promise<PickedMedia[]> { return unavailable(); }
export async function capturePhoto(_options?: CapturePhotoOptions): Promise<PickedMedia | null> { return unavailable(); }
export async function captureVideo(_options?: CaptureVideoOptions): Promise<PickedMedia | null> { return unavailable(); }
export async function cleanupCache() {}
