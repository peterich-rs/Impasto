import {
  checkPermissionStatus,
  checkServiceStatus,
  requestPermissions,
  shouldShowRequestPermissionRationale,
  type PermissionStatus,
  type ServiceStatus,
} from "@impasto/permissions/native";

export { openAppSettings, type PermissionStatus, type ServiceStatus } from "@impasto/permissions/native";

/** Names follow permission_handler 11.3.1. The scene maps each name. */
const NAMES = [
  "camera",
  "microphone",
  "speech",
  "location",
  "locationWhenInUse",
  "locationAlways",
  "contacts",
  "storage",
  "photos",
  "notification",
  "bluetooth",
  "bluetoothConnect",
  "bluetoothScan",
  "calendar",
] as const;

export type PermissionHandle = {
  readonly name: string;
  readonly status: Promise<PermissionStatus>;
  readonly shouldShowRequestRationale: Promise<boolean>;
  readonly serviceStatus: Promise<ServiceStatus>;
  request(): Promise<PermissionStatus>;
};

function bind(name: string): PermissionHandle {
  return {
    name,
    get status() {
      return checkPermissionStatus(name);
    },
    get shouldShowRequestRationale() {
      return shouldShowRequestPermissionRationale(name);
    },
    get serviceStatus() {
      return checkServiceStatus(name);
    },
    request() {
      return request([this]).then((result) => result[name] ?? "denied");
    },
  };
}

function table(): Record<string, PermissionHandle> {
  const out: Record<string, PermissionHandle> = {};
  for (const name of NAMES) {
    out[name] = bind(name);
  }
  return out;
}

export const Permission = table();

export async function request(
  permissions: readonly PermissionHandle[],
): Promise<Record<string, PermissionStatus>> {
  return requestPermissions(permissions.map((item) => item.name));
}
