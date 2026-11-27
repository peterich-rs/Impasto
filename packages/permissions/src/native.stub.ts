import type { NativePermissionName, NotificationPermissionOptions, PermissionStatus, ServiceStatus } from "./types";

export function isPermissionHostAvailable(): boolean { return false; }

export async function checkPermissionStatus(_permission: NativePermissionName): Promise<PermissionStatus> {
  return "denied";
}

export async function requestPermissions(names: NativePermissionName[]): Promise<Record<string, PermissionStatus>> {
  return Object.fromEntries(names.map((name) => [name, "denied"]));
}

export async function requestNotification(_options: NotificationPermissionOptions): Promise<PermissionStatus> {
  return "denied";
}

export async function shouldShowRequestPermissionRationale(_permission: NativePermissionName): Promise<boolean> {
  return false;
}

export async function checkServiceStatus(_permission: NativePermissionName): Promise<ServiceStatus> {
  return "notApplicable";
}

export async function openAppSettings(): Promise<boolean> { return false; }
export async function openSystemSettings(_permission?: NativePermissionName): Promise<boolean> { return false; }
export async function presentLimitedLibraryPicker(): Promise<boolean> { return false; }
