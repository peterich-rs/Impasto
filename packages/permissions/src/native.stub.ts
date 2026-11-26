import type { PermissionStatus, ServiceStatus } from "@impasto/permissions/native";

export function checkPermissionStatus(_permission: string): Promise<PermissionStatus> {
  return Promise.resolve("denied");
}

export function requestPermissions(
  permissions: string[],
): Promise<Record<string, PermissionStatus>> {
  const result: Record<string, PermissionStatus> = {};
  for (const name of permissions) {
    result[name] = "denied";
  }
  return Promise.resolve(result);
}

export function shouldShowRequestPermissionRationale(
  _permission: string,
): Promise<boolean> {
  return Promise.resolve(false);
}

export function checkServiceStatus(_permission: string): Promise<ServiceStatus> {
  return Promise.resolve("notApplicable");
}

export function openAppSettings(): Promise<boolean> {
  return Promise.resolve(false);
}
