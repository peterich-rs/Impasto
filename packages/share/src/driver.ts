import * as native from "@impasto/share/native";

export type Driver = Pick<typeof native, "share">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isShareHostAvailable() || mock) throw new Error("A share mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
