export function multiGet(_keys: string[]): Promise<[string, string | null][]> {
  return Promise.resolve([]);
}

export function multiSet(_entries: [string, string][]): Promise<void> {
  return Promise.resolve();
}

export function multiRemove(_keys: string[]): Promise<void> {
  return Promise.resolve();
}

export function clear(): Promise<void> {
  return Promise.resolve();
}

export function addEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}

export function removeEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}
