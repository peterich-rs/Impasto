import type { SqlValue } from "./types";

export interface SqlFragment { readonly text: string; readonly params: readonly SqlValue[]; }

export function sql(strings: TemplateStringsArray, ...values: readonly SqlValue[]): SqlFragment {
  return { text: strings.join("?"), params: values };
}

function field(name: string): string {
  if (!name || name.includes("\0")) throw new TypeError("Invalid SQLite identifier");
  return `"${name.replace(/"/g, '""')}"`;
}
function predicates(items: readonly SqlFragment[]): string {
  return items.length ? ` WHERE ${items.map(filter => `(${filter.text})`).join(" AND ")}` : "";
}
function entries<T>(values: Partial<T>): [string, SqlValue][] {
  const fields = Object.entries(values) as [string, SqlValue][];
  if (!fields.length || fields.some(([, value]) => value === undefined)) throw new TypeError("Expected SQL column values");
  return fields;
}

type Order = "asc" | "desc";
export interface SelectBuilder<T> {
  where(filter: SqlFragment): SelectBuilder<T>;
  orderBy(column: keyof T & string, order?: Order): SelectBuilder<T>;
  limit(count: number, offset?: number): SelectBuilder<T>;
  build(): SqlFragment;
}
export interface UpdateBuilder<T> {
  where(filter: SqlFragment): UpdateBuilder<T>;
  build(): SqlFragment;
}

export function selectFrom<T>(table: string, columns?: readonly (keyof T & string)[]): SelectBuilder<T> {
  const filters: SqlFragment[] = [];
  const ordering: string[] = [];
  let window: { count: number; offset: number } | undefined;
  return {
    where(filter) { filters.push(filter); return this; },
    orderBy(column, order = "asc") {
      if (order !== "asc" && order !== "desc") throw new TypeError("Invalid SQL sort order");
      ordering.push(`${field(column)} ${order === "asc" ? "ASC" : "DESC"}`);
      return this;
    },
    limit(count, offset = 0) {
      if (![count, offset].every(value => Number.isSafeInteger(value) && value >= 0)) throw new RangeError("Invalid SQL limit");
      window = { count, offset };
      return this;
    },
    build() {
      let text = `SELECT ${columns?.length ? columns.map(field).join(", ") : "*"} FROM ${field(table)}${predicates(filters)}`;
      if (ordering.length) text += ` ORDER BY ${ordering.join(", ")}`;
      const params = filters.flatMap(filter => filter.params);
      if (window) { text += " LIMIT ? OFFSET ?"; params.push(window.count, window.offset); }
      return { text, params };
    },
  };
}

export function insertInto<T>(table: string, row: Partial<T>): SqlFragment {
  const fields = entries(row);
  return {
    text: `INSERT INTO ${field(table)} (${fields.map(([key]) => field(key)).join(", ")}) VALUES (${fields.map(() => "?").join(", ")})`,
    params: fields.map(([, value]) => value),
  };
}

export function updateTable<T>(table: string, patch: Partial<T>): UpdateBuilder<T> {
  const fields = entries(patch);
  const filters: SqlFragment[] = [];
  return {
    where(filter) { filters.push(filter); return this; },
    build() {
      return { text: `UPDATE ${field(table)} SET ${fields.map(([key]) => `${field(key)} = ?`).join(", ")}${predicates(filters)}`,
        params: [...fields.map(([, value]) => value), ...filters.flatMap(filter => filter.params)] };
    },
  };
}
