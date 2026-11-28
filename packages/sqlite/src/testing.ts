import { sqliteDriver } from "./driver";
import { openDatabase } from "./index";

/** Opens a private in-memory database for a headless test. Call close() during cleanup. */
export function createTestDatabase() {
  if (sqliteDriver.isSqliteHostAvailable()) throw new Error("[sqlite:host-unavailable] A test database requires an unbound test host");
  return openDatabase({ name: ":memory:" });
}
