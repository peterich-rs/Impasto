export type HeadersInit = Headers | [string, string][] | Record<string, string>;

const INVALID_HEADER_NAME = /[^!#$%&'*+\-.^_`|~0-9A-Za-z]/;
const INVALID_HEADER_VALUE = /[\r\n\0]/;

function normalizeName(name: string): string {
  const key = String(name);
  if (key.length === 0 || INVALID_HEADER_NAME.test(key)) {
    throw new TypeError(`Invalid HTTP header name: ${key}`);
  }
  return key.toLowerCase();
}

function normalizeValue(value: string): string {
  const text = String(value).replace(/^[ \t]+|[ \t]+$/g, "");
  if (INVALID_HEADER_VALUE.test(text)) {
    throw new TypeError("Invalid HTTP header value");
  }
  return text;
}

export class Headers {
  readonly #map = new Map<string, string[]>();

  constructor(init?: HeadersInit) {
    if (!init) return;
    if (init instanceof Headers) {
      for (const [name, value] of init.entries()) {
        this.append(name, value);
      }
      return;
    }
    if (Array.isArray(init)) {
      for (const pair of init) {
        if (!Array.isArray(pair) || pair.length !== 2) {
          throw new TypeError("Headers tuple must be [name, value]");
        }
        this.append(pair[0], pair[1]);
      }
      return;
    }
    if (Symbol.iterator in init) {
      for (const pair of init as Iterable<[string, string]>) {
        this.append(pair[0], pair[1]);
      }
      return;
    }
    for (const [name, value] of Object.entries(init)) {
      this.append(name, value);
    }
  }

  append(name: string, value: string): void {
    const key = normalizeName(name);
    const val = normalizeValue(value);
    const list = this.#map.get(key);
    if (list) {
      list.push(val);
    } else {
      this.#map.set(key, [val]);
    }
  }

  set(name: string, value: string): void {
    const key = normalizeName(name);
    this.#map.set(key, [normalizeValue(value)]);
  }

  delete(name: string): void {
    this.#map.delete(normalizeName(name));
  }

  get(name: string): string | null {
    const values = this.#map.get(normalizeName(name));
    return values ? values.join(", ") : null;
  }

  has(name: string): boolean {
    return this.#map.has(normalizeName(name));
  }

  getSetCookie(): string[] {
    return [...(this.#map.get("set-cookie") ?? [])];
  }

  *entries(): IterableIterator<[string, string]> {
    const keys = [...this.#map.keys()].sort();
    for (const key of keys) {
      if (key === "set-cookie") {
        for (const cookie of this.#map.get(key) ?? []) {
          yield [key, cookie];
        }
      } else {
        yield [key, (this.#map.get(key) ?? []).join(", ")];
      }
    }
  }

  *keys(): IterableIterator<string> {
    for (const [key] of this.entries()) {
      yield key;
    }
  }

  *values(): IterableIterator<string> {
    for (const [, value] of this.entries()) {
      yield value;
    }
  }

  [Symbol.iterator](): IterableIterator<[string, string]> {
    return this.entries();
  }

  forEach(
    callback: (value: string, key: string, parent: this) => void,
    thisArg?: unknown,
  ): void {
    for (const [key, value] of this.entries()) {
      callback.call(thisArg, value, key, this);
    }
  }

  toPairs(): [string, string][] {
    const pairs: [string, string][] = [];
    for (const [name, list] of this.#map.entries()) {
      for (const value of list) {
        pairs.push([name, value]);
      }
    }
    return pairs;
  }
}
