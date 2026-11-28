import * as native from "@impasto/biometrics/native";
type Driver = Pick<typeof native, "getCapabilities" | "authenticate" | "cancel" | "nextPromptId">;
let mock: Driver | undefined;
export function driver(): Driver { return mock ?? native; }
export function installMock(value: Driver): () => void {
  if (native.isBiometricsHostAvailable() || mock) throw new Error("[biometrics:host-unavailable] Mock requires a test host");
  mock = value;
  return () => { if (mock === value) mock = undefined; };
}
