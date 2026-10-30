export function documentDirectory(): string {
  return "";
}

export function read(_path: string): Promise<Uint8Array> {
  return Promise.reject(new Error("FileSystem is not linked"));
}

export function write(_path: string, _bytes: Uint8Array): Promise<void> {
  return Promise.reject(new Error("FileSystem is not linked"));
}

function deletePath(_path: string): Promise<void> {
  return Promise.reject(new Error("FileSystem is not linked"));
}

export { deletePath as delete };

export function pick(): Promise<{ name: string } | null> {
  return Promise.resolve(null);
}

export function addEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}

export function removeEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}
