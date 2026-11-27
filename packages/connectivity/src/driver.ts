import * as native from "@impasto/connectivity/native";

export type Driver = Pick<typeof native, "getStatus" | "watchStatus">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isConnectivityHostAvailable() || mock) throw new Error("A connectivity mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
