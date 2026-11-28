import * as native from "@impasto/geolocator/native";
type Driver = Pick<typeof native, "getCurrentPosition" | "nextRequestId" | "cancel" | "getLastKnownPosition" | "watchPosition" |
  "isLocationServiceEnabled" | "watchServiceStatus" | "checkPermission" | "requestPermission">;
let mock: Driver | undefined;
export function driver(): Driver { return mock ?? native; }
export function installMock(value: Driver): () => void {
  if (native.isGeolocatorHostAvailable() || mock) throw new Error("[geolocator:host-unavailable] Mock requires a test host");
  mock = value;
  return () => { if (mock === value) mock = undefined; };
}
export { native };
