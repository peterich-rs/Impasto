/** Not registered by the runtime. A host plugin implements this name. */
export interface AsyncStorage {
  multiGet(keys: string[]): Promise<[string, string | null][]>;
  multiSet(entries: [string, string][]): Promise<void>;
  multiRemove(keys: string[]): Promise<void>;
  clear(): Promise<void>;
}

export const name = "AsyncStorage";
