export function isSqliteHostAvailable() { return false; }
export async function openDatabase(_options: unknown): Promise<never> {
  throw new Error("[sqlite:closed] Impasto native SQLite is unavailable");
}
export async function deleteDatabase(_name: string, _scope?: string): Promise<void> {
  throw new Error("[sqlite:closed] Impasto native SQLite is unavailable");
}
