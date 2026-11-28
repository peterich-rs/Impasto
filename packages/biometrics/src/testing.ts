import { installMock } from "./driver";
import type { AuthenticateOptions, BiometricAuthResult, BiometricCapabilities } from "./types";

export function mockBiometrics(options: { capabilities?: BiometricCapabilities; nextResult?: BiometricAuthResult } = {}) {
  let next = options.nextResult ?? { authenticated: true };
  let id = 0;
  const calls: AuthenticateOptions[] = [];
  const restore = installMock({
    async getCapabilities() {
      const capabilities = options.capabilities;
      return capabilities
        ? { ...capabilities, biometryTypes: [...capabilities.biometryTypes] }
        : { isAvailable: true, isEnrolled: true, hasDeviceCredential: true,
            biometryTypes: ["fingerprint"], securityLevel: "strong" };
    },
    nextPromptId: () => ++id,
    async authenticate(_id, config) {
      calls.push({ reason: config.reason, title: config.title ?? undefined, subtitle: config.subtitle ?? undefined,
        description: config.description ?? undefined, allowDeviceCredential: config.allowDeviceCredential ?? undefined,
        allowWatch: config.allowWatch ?? undefined, cancelLabel: config.cancelLabel ?? undefined,
        fallbackLabel: config.fallbackLabel ?? undefined, confirmationRequired: config.confirmationRequired ?? undefined });
      return { ...next };
    },
    async cancel() {},
  });
  return { calls, respondWith(result: BiometricAuthResult) { next = result; }, restore };
}
