import {
  checkPermissionStatus,
  checkServiceStatus,
  isPermissionHostAvailable,
  openAppSettings,
  openSystemSettings,
  presentLimitedLibraryPicker,
  requestNotification,
  requestPermissions,
  shouldShowRequestPermissionRationale,
} from "@impasto/permissions/native";

import type { NativePermissionName, NotificationPermissionOptions, PermissionStatus, ServiceStatus } from "./types";

export interface PermissionDriver {
  checkPermissionStatus(name: NativePermissionName): Promise<PermissionStatus>;
  checkServiceStatus(name: NativePermissionName): Promise<ServiceStatus>;
  requestPermissions(names: NativePermissionName[]): Promise<Record<string, PermissionStatus>>;
  requestNotification(options: NotificationPermissionOptions): Promise<PermissionStatus>;
  shouldShowRequestPermissionRationale(name: NativePermissionName): Promise<boolean>;
  openAppSettings(): Promise<boolean>;
  openSystemSettings(name?: NativePermissionName): Promise<boolean>;
  presentLimitedLibraryPicker(): Promise<boolean>;
}

const native: PermissionDriver = {
  checkPermissionStatus,
  checkServiceStatus,
  requestPermissions,
  requestNotification,
  shouldShowRequestPermissionRationale,
  openAppSettings,
  openSystemSettings,
  presentLimitedLibraryPicker,
};
let mock: PermissionDriver | undefined;

export function driver(): PermissionDriver {
  return mock ?? native;
}

export function installMock(replacement: PermissionDriver): () => void {
  if (isPermissionHostAvailable()) throw new Error("Permission mocks require a headless test host");
  if (mock) throw new Error("A permission mock is already installed");
  mock = replacement;
  return () => {
    if (mock === replacement) mock = undefined;
  };
}
