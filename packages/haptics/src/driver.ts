import * as native from "@impasto/haptics/native";

export type Driver = Pick<typeof native, "isSupported" | "impact" | "selection" | "notification" | "vibrate" | "vibratePattern" | "cancel">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isHapticsHostAvailable() || mock) throw new Error("A haptics mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
