export const SERVICE_NAMES = [
  "location", "locationWhenInUse", "locationAlways", "bluetooth", "bluetoothScan",
  "bluetoothConnect", "bluetoothAdvertise", "notification", "phone",
] as const;

export const STANDARD_NAMES = [
  "camera", "microphone", "speech", "photos", "photosAddOnly", "videos", "audio",
  "mediaLibrary", "contacts", "calendarWriteOnly", "calendarFullAccess", "calendar",
  "reminders", "storage", "manageExternalStorage", "accessMediaLocation", "nearbyWifiDevices",
  "sensors", "sensorsAlways", "activityRecognition", "criticalAlerts", "scheduleExactAlarm",
  "accessNotificationPolicy", "systemAlertWindow", "requestInstallPackages",
  "ignoreBatteryOptimizations", "sms", "appTrackingTransparency", "assistant",
  "backgroundRefresh", "screenCapture", "accessibility", "fullDiskAccess",
] as const;

export type ServicePermissionName = typeof SERVICE_NAMES[number];
export type StandardPermissionName = typeof STANDARD_NAMES[number];
export type PermissionName = ServicePermissionName | StandardPermissionName;
export type NativePermissionName = Exclude<PermissionName, "calendar">;

export type PermissionStatus =
  | "denied" | "granted" | "restricted" | "limited" | "permanentlyDenied" | "provisional";
export type ServiceStatus = "enabled" | "disabled" | "notApplicable";
export type PermissionErrorCode =
  | "not-enabled" | "missing-config" | "already-requesting" | "host-unavailable" | "cancelled" | "platform";

export interface NotificationPermissionOptions {
  alert?: boolean;
  badge?: boolean;
  sound?: boolean;
  provisional?: boolean;
  criticalAlert?: boolean;
}
