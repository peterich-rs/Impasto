/**
 * vConsole-style install. No-op on hosts with devtools disabled, so release
 * builds keep exactly the same behavior.
 */
import { registerTopLayer, type NodeHandle } from "@impasto/solid-renderer";
import { DevtoolsLayer } from "./layer.tsx";
import { store } from "./store.ts";

export type DevtoolsHandle = {
  readonly available: boolean;
  uninstall(): void;
};

export function installDevtools(): DevtoolsHandle {
  const handle: DevtoolsHandle = {
    available: store.available(),
    uninstall() {},
  };
  if (!handle.available) {
    return handle;
  }
  const release = registerTopLayer(() => DevtoolsLayer() as NodeHandle);
  handle.uninstall = () => {
    release();
  };
  return handle;
}
