import * as native from "@impasto/notifications/native";
type Driver = Pick<typeof native, "show" | "schedule" | "invoke" | "checkPermission" | "requestPermission" |
  "watchResponses" | "watchForeground">;
let mock: Driver | undefined;
export function driver(): Driver { return mock ?? native; }
export function installMock(value: Driver): () => void {
  if (native.isNotificationsHostAvailable() || mock) throw new Error("[notifications:host-unavailable] Mock requires a test host");
  mock = value;
  return () => { if (mock === value) mock = undefined; };
}
