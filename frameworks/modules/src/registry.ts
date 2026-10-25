export interface ModuleRegistry {
  get<T>(name: string): T | null;
  getEnforcing<T>(name: string): T;
}

/** The registry installed by the QuickJS prelude. */
export const TurboModuleRegistry: ModuleRegistry = (
  globalThis as { TurboModuleRegistry: ModuleRegistry }
).TurboModuleRegistry;
