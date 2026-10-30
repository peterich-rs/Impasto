import * as host from "@impasto/async-storage/host";

export interface AsyncStorage {
  multiGet(keys: string[]): Promise<[string, string | null][]>;
  multiSet(entries: [string, string][]): Promise<void>;
  multiRemove(keys: string[]): Promise<void>;
  clear(): Promise<void>;
  addEventListener(type: string, listener: (e: unknown) => void): void;
  removeEventListener(type: string, listener: (e: unknown) => void): void;
}

export const AsyncStorage = host as unknown as AsyncStorage;
