import type { Permissions as PermissionsHost } from "@uifw/modules";

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

type Registry = {
  getEnforcing<T>(name: string): T;
};

function host(): PermissionsHost {
  const registry = (globalThis as { TurboModuleRegistry?: Registry }).TurboModuleRegistry;
  if (!registry) {
    throw new Error('module "Permissions" is not linked');
  }
  return registry.getEnforcing<PermissionsHost>("Permissions");
}

function bind(name: string): PermissionHandle {
  return {
    name,
    get status() {
      return host().checkPermissionStatus(name) as Promise<PermissionStatus>;
    },
    get shouldShowRequestRationale() {
      return host().shouldShowRequestPermissionRationale(name);
    },
    get serviceStatus() {
      return host().checkServiceStatus(name) as Promise<ServiceStatus>;
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
  const result = await host().requestPermissions(permissions.map((item) => item.name));
  return result as Record<string, PermissionStatus>;
}

export function openAppSettings(): Promise<boolean> {
  return host().openAppSettings();
}
