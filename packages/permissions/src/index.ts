import {
  checkPermissionStatus,
  checkServiceStatus,
  openAppSettings as hostOpenAppSettings,
  requestPermissions,
  shouldShowRequestPermissionRationale,
} from "@impasto/permissions/host";

/** Names follow permission_handler 11.3.1. The host maps each name. */
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

export type PermissionStatus =
  | "granted"
  | "denied"
  | "restricted"
  | "permanentlyDenied"
  | "limited"
  | "provisional";

export type ServiceStatus = "enabled" | "disabled" | "notApplicable";

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
      return checkPermissionStatus(name) as Promise<PermissionStatus>;
    },
    get shouldShowRequestRationale() {
      return shouldShowRequestPermissionRationale(name) as Promise<boolean>;
    },
    get serviceStatus() {
      return checkServiceStatus(name) as Promise<ServiceStatus>;
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
  const result = await requestPermissions(permissions.map((item) => item.name));
  return result as Record<string, PermissionStatus>;
}

export function openAppSettings(): Promise<boolean> {
  return hostOpenAppSettings() as Promise<boolean>;
}
