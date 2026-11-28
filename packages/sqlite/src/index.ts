import type { DirectoryScope } from "@impasto/file-system";
import { sqliteDriver as native } from "./driver";
import type { Database, ExecuteResult, OpenDatabaseOptions, SqlExecutor, SqlParams, SqlValue, Transaction } from "./types";
export type * from "./types";

type WireValue = number | string | null | Uint8Array | ArrayBuffer | { integer: string };
type WireParams = { positional?: WireValue[]; named?: [string, WireValue][] };
type WireResult = { changes: number; lastInsertRowid: WireValue };

function parameter(value: SqlValue): WireValue {
  return typeof value === "bigint" ? { integer: value.toString() } : value;
}
function parameters(values?: SqlParams): WireParams | undefined {
  if (values === undefined) return undefined;
  if (Array.isArray(values)) return { positional: values.map(parameter) };
  return { named: Object.entries(values).map(([key, value]) => [key, parameter(value)]) };
}
function value(wire: WireValue): SqlValue {
  if (wire === null || typeof wire !== "object" || wire instanceof Uint8Array) return wire;
  if (wire instanceof ArrayBuffer) return new Uint8Array(wire);
  return BigInt(wire.integer);
}
function result(wire: WireResult): ExecuteResult {
  const id = value(wire.lastInsertRowid);
  if (typeof id !== "number" && typeof id !== "bigint") throw new TypeError("Invalid SQLite rowid");
  return { changes: wire.changes, lastInsertRowid: id };
}
interface NativeExecutor {
  query(sql: string, params?: WireParams): Promise<[string, WireValue][][]>;
  execute(sql: string, params?: WireParams): Promise<WireResult>;
}

function executor(handle: NativeExecutor): SqlExecutor {
  return {
    async query<T extends Record<string, SqlValue>>(sql: string, params?: SqlParams): Promise<T[]> {
      return (await handle.query(sql, parameters(params))).map(row =>
        Object.fromEntries(row.map(([column, cell]) => [column, value(cell)])) as T);
    },
    async queryOne<T extends Record<string, SqlValue>>(sql: string, params?: SqlParams): Promise<T | null> {
      const rows = await this.query<T>(sql, params);
      return rows[0] ?? null;
    },
    async execute(sql: string, params?: SqlParams) { return result(await handle.execute(sql, parameters(params))); },
  };
}

export async function openDatabase(options: OpenDatabaseOptions): Promise<Database> {
  const handle = await native.openDatabase(options);
  return {
    ...executor(handle),
    async batch(statements) {
      const results = await handle.batch(statements.map(item => ({ sql: item.sql, params: parameters(item.params) })));
      return results.map(result);
    },
    async transaction(fn) {
      const lease = await handle.beginTransaction();
      const tx: Transaction = {
        ...executor(lease),
        async savepoint(task) {
          const point = await lease.savepoint();
          try {
            const result = await task(tx);
            await lease.releaseSavepoint(point, false);
            return result;
          } catch (error) {
            await lease.releaseSavepoint(point, true).catch(() => undefined);
            throw error;
          }
        },
      };
      try {
        const result = await fn(tx);
        await lease.finish(true);
        return result;
      } catch (error) {
        await lease.finish(false).catch(() => undefined);
        throw error;
      }
    },
    migrate: migrations => handle.migrate(migrations.map(migration => ({ ...migration, up: [...migration.up] }))),
    checkpoint: mode => handle.checkpoint(mode),
    async close() { await handle.dispose(); handle.close(); },
  };
}
export const deleteDatabase = (name: string, scope?: DirectoryScope) => native.deleteDatabase(name, scope);
