import type { AuthenticateOptions, BiometricAuthResult, BiometricCapabilities } from "./types";
export const isBiometricsHostAvailable = () => false;
export const nextPromptId = () => 0;
export async function getCapabilities(): Promise<BiometricCapabilities> { throw unavailable(); }
export async function authenticate(_id: number, _options: Omit<AuthenticateOptions, "signal">): Promise<BiometricAuthResult> { throw unavailable(); }
export async function cancel(_id: number): Promise<void> {}
function unavailable() { return new Error("[biometrics:host-unavailable] Biometrics requires a native host"); }
