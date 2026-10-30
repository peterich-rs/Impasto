import type { CoreHost } from "../src/host.ts";

/**
 * QuickJS installs `core` on the global object before evaluating the bundle.
 * Lookup is lazy so a test can install the mock after the module is imported,
 * and a missing method reads as `undefined` instead of throwing on access.
 */
export const host: CoreHost = new Proxy({} as CoreHost, {
  get(_target, prop) {
    if (typeof prop !== "string") {
      return undefined;
    }
    const core = (globalThis as { core?: Partial<CoreHost> }).core;
    const value = core?.[prop as keyof CoreHost];
    if (typeof value === "function") {
      return (value as (...args: unknown[]) => unknown).bind(core);
    }
    return value;
  },
});
