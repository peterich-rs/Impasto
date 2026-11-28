import { driver, installMock } from "./driver";
import type { AppInfo, DeviceInfo, RuntimeInfo } from "./types";
export function mockSystemInfo(initial: { app?: Partial<AppInfo>; device?: Partial<DeviceInfo>; runtime?: Partial<RuntimeInfo> } = {}) {
  const native = driver();
  let runtime = initial.runtime;
  const restore = installMock({
    async getAppInfo() { return { ...await native.getAppInfo(), ...initial.app }; },
    async getDeviceInfo() { return { ...await native.getDeviceInfo(), ...initial.device }; },
    async getRuntimeInfo() { return { ...await native.getRuntimeInfo(), ...runtime }; },
  });
  return { setRuntime(value: Partial<RuntimeInfo>) { runtime = { ...runtime, ...value }; }, restore };
}
