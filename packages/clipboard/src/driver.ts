import * as native from "@impasto/clipboard/native";

export type Driver = Pick<typeof native, "getString" | "setString" | "hasStrings" | "hasImages" | "hasHtml" | "availableFormats" | "read" | "write" | "clear" | "watchChanges">;
let mock: Driver | undefined;

export function driver(): Driver { return mock ?? native; }

export function installMock(replacement: Driver): () => void {
  if (native.isClipboardHostAvailable() || mock) throw new Error("A clipboard mock requires an unbound test host");
  mock = replacement;
  return () => { if (mock === replacement) mock = undefined; };
}
