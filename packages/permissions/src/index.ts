import { driver } from "./driver";
import {
  SERVICE_NAMES, STANDARD_NAMES,
  type NativePermissionName, type NotificationPermissionOptions, type PermissionName,
  type PermissionStatus, type ServicePermissionName, type ServiceStatus, type StandardPermissionName,
} from "./types";

export type {
  NotificationPermissionOptions, PermissionErrorCode, PermissionName, PermissionStatus,
  ServicePermissionName, ServiceStatus, StandardPermissionName,
} from "./types";

export interface PermissionCallbacks {
  onGranted?: () => void | Promise<void>;
  onDenied?: () => void | Promise<void>;
  onPermanentlyDenied?: () => void | Promise<void>;
  onRestricted?: () => void | Promise<void>;
  onLimited?: () => void | Promise<void>;
  onProvisional?: () => void | Promise<void>;
}

export interface PermissionHandle<N extends PermissionName = PermissionName> {
  readonly name: N;
  readonly status: Promise<PermissionStatus>;
  readonly isGranted: Promise<boolean>;
  readonly isDenied: Promise<boolean>;
  readonly isPermanentlyDenied: Promise<boolean>;
  readonly isRestricted: Promise<boolean>;
  readonly isLimited: Promise<boolean>;
  readonly isProvisional: Promise<boolean>;
  readonly shouldShowRequestRationale: Promise<boolean>;
  readonly serviceStatus: Promise<ServiceStatus>;
  request(callbacks?: PermissionCallbacks): Promise<PermissionStatus>;
}

export interface PermissionWithServiceHandle<N extends ServicePermissionName = ServicePermissionName>
  extends PermissionHandle<N> {
  readonly isServiceEnabled: Promise<boolean>;
}

export type PermissionTable = {
  readonly [K in ServicePermissionName]: PermissionWithServiceHandle<K>;
} & {
  readonly [K in Exclude<StandardPermissionName, "calendar">]: PermissionHandle<K>;
} & {
  /** @deprecated Use calendarFullAccess. */
  readonly calendar: PermissionHandle<"calendar">;
};

function nativeName(name: PermissionName): NativePermissionName {
  return name === "calendar" ? "calendarFullAccess" : name;
}

export const isGranted = (status: PermissionStatus): boolean => status === "granted";
export const isDenied = (status: PermissionStatus): boolean => status === "denied";
export const isPermanentlyDenied = (status: PermissionStatus): boolean => status === "permanentlyDenied";
export const isRestricted = (status: PermissionStatus): boolean => status === "restricted";
export const isLimited = (status: PermissionStatus): boolean => status === "limited";
export const isProvisional = (status: PermissionStatus): boolean => status === "provisional";
export const isServiceEnabled = (status: ServiceStatus): boolean => status === "enabled";

const callbacksByStatus = {
  granted: "onGranted",
  denied: "onDenied",
  permanentlyDenied: "onPermanentlyDenied",
  restricted: "onRestricted",
  limited: "onLimited",
  provisional: "onProvisional",
} as const satisfies Record<PermissionStatus, keyof PermissionCallbacks>;

async function notify(status: PermissionStatus, callbacks?: PermissionCallbacks): Promise<PermissionStatus> {
  await callbacks?.[callbacksByStatus[status]]?.();
  return status;
}

function bind<N extends PermissionName>(name: N, service: boolean): PermissionHandle<N> {
  const wire = nativeName(name);
  const handle: PermissionHandle<N> = {
    name,
    get status() { return driver().checkPermissionStatus(wire); },
    get isGranted() { return this.status.then(isGranted); },
    get isDenied() { return this.status.then(isDenied); },
    get isPermanentlyDenied() { return this.status.then(isPermanentlyDenied); },
    get isRestricted() { return this.status.then(isRestricted); },
    get isLimited() { return this.status.then(isLimited); },
    get isProvisional() { return this.status.then(isProvisional); },
    get shouldShowRequestRationale() { return driver().shouldShowRequestPermissionRationale(wire); },
    get serviceStatus() {
      return service ? driver().checkServiceStatus(wire) : Promise.resolve<ServiceStatus>("notApplicable");
    },
    async request(callbacks) {
      const result = await driver().requestPermissions([wire]);
      return notify(result[wire], callbacks);
    },
  };
  if (service) {
    Object.defineProperty(handle, "isServiceEnabled", { get: () => handle.serviceStatus.then(isServiceEnabled) });
  }
  return handle;
}

export const Permission = Object.fromEntries([
  ...SERVICE_NAMES.map((name) => [name, bind(name, true)]),
  ...STANDARD_NAMES.map((name) => [name, bind(name, false)]),
]) as PermissionTable;

type Statuses<T extends readonly PermissionHandle[]> = { [K in T[number]["name"]]: PermissionStatus };

export async function request<const T extends readonly PermissionHandle[]>(permissions: T): Promise<Statuses<T>> {
  if (permissions.length === 0) return {} as Statuses<T>;
  const names = [...new Set(permissions.map(({ name }) => nativeName(name)))];
  const statuses = await driver().requestPermissions(names);
  return Object.fromEntries(permissions.map(({ name }) => [name, statuses[nativeName(name)]])) as Statuses<T>;
}

export async function checkStatuses<const T extends readonly PermissionHandle[]>(permissions: T): Promise<Statuses<T>> {
  const statuses = await Promise.all(permissions.map(async (permission) => [permission.name, await permission.status] as const));
  return Object.fromEntries(statuses) as Statuses<T>;
}

export async function requestNotification(
  options: NotificationPermissionOptions & PermissionCallbacks = {},
): Promise<PermissionStatus> {
  const { alert, badge, sound, provisional, criticalAlert } = options;
  return notify(await driver().requestNotification({ alert, badge, sound, provisional, criticalAlert }), options);
}

export function openAppSettings(): Promise<boolean> {
  return driver().openAppSettings();
}

export function openSystemSettings(permission?: PermissionHandle): Promise<boolean> {
  return permission ? driver().openSystemSettings(nativeName(permission.name)) : openAppSettings();
}

export function presentLimitedLibraryPicker(): Promise<boolean> {
  return driver().presentLimitedLibraryPicker();
}
