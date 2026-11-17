export function checkPermissionStatus(_permission: string): Promise<string> {
  return Promise.resolve("denied");
}

export function requestPermissions(
  permissions: string[],
): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
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

export function checkServiceStatus(_permission: string): Promise<string> {
  return Promise.resolve("notApplicable");
}

export function openAppSettings(): Promise<boolean> {
  return Promise.resolve(false);
}

export function addEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}

export function removeEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}
