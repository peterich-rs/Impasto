import * as native from "@impasto/image-picker/native";
type Driver = Pick<typeof native, "pickMedia" | "capturePhoto" | "captureVideo" | "cleanupCache">;
let mock: Driver | undefined;
export function driver(): Driver { return mock ?? native; }
export function installMock(value: Driver): () => void {
  if (native.isImagePickerHostAvailable() || mock) throw new Error("[image-picker:host-unavailable] Mock requires a test host");
  mock = value;
  return () => { if (mock === value) mock = undefined; };
}
