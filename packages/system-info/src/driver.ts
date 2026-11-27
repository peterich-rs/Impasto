import * as native from "@impasto/system-info/native";

export type Driver = Pick<typeof native, "getAppInfo" | "getDeviceInfo" | "getRuntimeInfo">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isSystemInfoHostAvailable() || mock) throw new Error("A system-info mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
