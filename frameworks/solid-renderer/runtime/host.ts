import type { CoreHost } from "../src/host.ts";

/** QuickJS installs `core` on the global object before evaluating the bundle. */
export const host: CoreHost = (globalThis as { core: CoreHost }).core;
