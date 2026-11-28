import type { DirectoryScope } from "@impasto/file-system";

export type SqlValue = null | number | bigint | string | Uint8Array;
export type SqlParams = readonly SqlValue[] | Record<string, SqlValue>;

export type SqliteErrorCode =
  | "busy"
  | "locked"
  | "constraint"
  | "syntax"
  | "mismatch"
  | "readonly"
  | "corrupt"
  | "not-found"
  | "migration-checksum"
  | "encryption-not-enabled"
  | "closed"
  | "outside-scope"
  | "io";

export interface OpenDatabaseOptions {
  /** 相对于 scope 下 `databases/` 的路径，或 ":memory:" */
  readonly name: string;
  readonly scope?: DirectoryScope; // 默认 "applicationSupport"
  readonly readOnly?: boolean;
  readonly wal?: boolean; // 默认 true（对文件库开启 WAL）
  readonly foreignKeys?: boolean; // 默认 true
  readonly busyTimeoutMs?: number; // 默认 5000
  /** WAL 下并发只读连接线程数，默认 2；":memory:" 时为 0（共享单连接） */
  readonly readPoolSize?: number;
  /** 超过 JS Number 安全整数范围时的处理策略，默认 "number"（超限转 string/错误由配置决定） */
  readonly int64Mode?: "number" | "bigint" | "string";
  /** 可选数据库加密密钥（需在 app.toml 开启 @impasto/sqlite 的 sqlcipher feature） */
  readonly encryptionKey?: string | Uint8Array;
}

export interface ExecuteResult {
  readonly changes: number;
  readonly lastInsertRowid: number | bigint;
}

export interface BatchStatement {
  readonly sql: string;
  readonly params?: SqlParams;
}

export interface SqliteMigration {
  readonly version: number; // 严格递增正整数：1, 2, 3...
  readonly name: string;
  readonly up: readonly string[];
}

export interface MigrationReport {
  readonly fromVersion: number;
  readonly toVersion: number;
  readonly applied: readonly { readonly version: number; readonly name: string }[];
}

export interface SqlExecutor {
  query<T extends Record<string, SqlValue> = Record<string, SqlValue>>(sql: string, params?: SqlParams): Promise<T[]>;
  queryOne<T extends Record<string, SqlValue> = Record<string, SqlValue>>(sql: string, params?: SqlParams): Promise<T | null>;
  execute(sql: string, params?: SqlParams): Promise<ExecuteResult>;
}

export interface Transaction extends SqlExecutor {
  savepoint<T>(fn: (tx: Transaction) => Promise<T>): Promise<T>;
}

export interface Database extends SqlExecutor {
  /** 单次 FFI 原子批量执行（包裹在隐式事务中） */
  batch(statements: readonly BatchStatement[]): Promise<ExecuteResult[]>;
  /** 获取 Writer 独占令牌并执行 IMMEDIATE 事务（throw 时自动 ROLLBACK） */
  transaction<T>(fn: (tx: Transaction) => Promise<T>): Promise<T>;
  migrate(migrations: readonly SqliteMigration[]): Promise<MigrationReport>;
  checkpoint(mode?: "passive" | "full" | "truncate" | "restart"): Promise<void>;
  close(): Promise<void>;
}
