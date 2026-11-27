export type PlatformName = "ios" | "macos" | "android" | "linux";
export type InstallerStore = "appStore" | "testFlight" | "googlePlay" | "amazon" | "huawei" |
  "xiaomi" | "sideload" | "simulator" | "unknown";
export type SystemInfoErrorCode = "host-unavailable" | "platform";
export interface AppInfo {
  readonly appName: string;
  readonly packageName: string;
  readonly version: string;
  readonly buildNumber: string;
  readonly installerStore: InstallerStore;
  readonly isDebug: boolean;
}
export interface DeviceInfo {
  readonly platform: PlatformName;
  readonly osVersion: string;
  readonly osBuild: string;
  readonly sdkInt: number | null;
  readonly model: string;
  readonly brand: string;
  readonly manufacturer: string;
  readonly architecture: string;
  readonly isPhysicalDevice: boolean;
  readonly processorCount: number;
  readonly totalMemoryBytes: number;
  readonly locale: string;
  readonly timeZone: string;
}
export interface RuntimeInfo {
  readonly freeDiskBytes: number;
  readonly totalDiskBytes: number;
  readonly isLowPowerMode: boolean;
}
