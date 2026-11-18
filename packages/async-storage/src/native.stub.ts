import type { AsyncStorage as AsyncStorageApi } from "./native.js";

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

export const AsyncStorage: AsyncStorageApi = Object.freeze({
  multiGet,
  multiSet,
  multiRemove,
  clear,
});
