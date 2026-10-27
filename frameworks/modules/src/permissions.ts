/**
 * Host object `Permissions`. Names match permission_handler's platform interface.
 * Unregistered hosts make `TurboModuleRegistry.get` return null.
 */
export interface Permissions {
  checkPermissionStatus(permission: string): Promise<string>;
  requestPermissions(permissions: string[]): Promise<Record<string, string>>;
  shouldShowRequestPermissionRationale(permission: string): Promise<boolean>;
  checkServiceStatus(permission: string): Promise<string>;
  openAppSettings(): Promise<boolean>;
}

export const name = "Permissions";
