import { invoke } from "./native.js";

export type NativePluginValue =
  | null
  | boolean
  | number
  | string
  | readonly NativePluginValue[]
  | { readonly [key: string]: NativePluginValue };

/** Calls a method on a native plugin registered with the current host. */
export async function invokeNativePlugin(
  identifier: string,
  method: string,
  value: NativePluginValue = null,
): Promise<NativePluginValue> {
  return JSON.parse(await invoke(identifier, method, JSON.stringify(value))) as NativePluginValue;
}
