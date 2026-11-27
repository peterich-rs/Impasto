import * as native from "@impasto/linking/native";

export type Driver = Pick<typeof native, "getInitialURL" | "canOpenURL" | "openURL" | "watchUrls" | "addEventListener" | "removeEventListener">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isLinkingHostAvailable() || mock) throw new Error("A linking mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
