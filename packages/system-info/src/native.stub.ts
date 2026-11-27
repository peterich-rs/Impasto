import type { AppInfo, DeviceInfo, RuntimeInfo } from "./types";

export function isSystemInfoHostAvailable() { return false; }
export const getAppInfo = async (): Promise<AppInfo> => ({
  appName: "", packageName: "", version: "", buildNumber: "", installerStore: "unknown", isDebug: false,
});
export const getDeviceInfo = async (): Promise<DeviceInfo> => ({
  platform: "linux", osVersion: "", osBuild: "", sdkInt: null, model: "", brand: "", manufacturer: "",
  architecture: "", isPhysicalDevice: false, processorCount: 1, totalMemoryBytes: 0, locale: "", timeZone: "",
});
export const getRuntimeInfo = async (): Promise<RuntimeInfo> => ({
  freeDiskBytes: 0, totalDiskBytes: 0, isLowPowerMode: false,
});
