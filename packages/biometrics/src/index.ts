import { driver } from "./driver";
import type { AuthenticateOptions, BiometricAuthResult, BiometricCapabilities } from "./types";
export type * from "./types";
export const getCapabilities = (): Promise<BiometricCapabilities> => driver().getCapabilities();
const cancelled = (): BiometricAuthResult => ({ authenticated: false, reason: "cancelled", message: "Authentication was cancelled" });
export async function authenticate({ signal, ...options }: AuthenticateOptions): Promise<BiometricAuthResult> {
  if (!options.reason.trim()) throw new TypeError("[biometrics:invalid-argument] An authentication reason is required");
  if (signal?.aborted) return cancelled();
  const native = driver();
  const id = native.nextPromptId();
  const pending = native.authenticate(id, options).then((result): BiometricAuthResult =>
    result.authenticated ? { authenticated: true }
      : { authenticated: false, reason: result.reason ?? "failed", message: result.message ?? "Authentication did not complete" });
  if (!signal) return pending;
  let abort = () => {};
  try {
    return await Promise.race([pending, new Promise<BiometricAuthResult>(resolve => {
      abort = () => { void native.cancel(id).catch(() => undefined); resolve(cancelled()); };
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
    })]);
  } finally { signal.removeEventListener("abort", abort); }
}
