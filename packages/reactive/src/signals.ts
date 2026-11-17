import { createMemo } from "solid-js";


type Source = Record<string | symbol, unknown> | (() => Record<string, unknown>);

export function merge(...sources: Source[]): Record<string, unknown> {
  const present = sources.filter((source) => source != null);
  if (present.length === 1 && typeof present[0] !== "function") {
    return present[0] as Record<string, unknown>;
  }
  const resolved = present.map((source) =>
    typeof source === "function" ? createMemo(source) : () => source,
  );
  return new Proxy(
    {},
    {
      get(_target, key) {
        for (let index = resolved.length - 1; index >= 0; index -= 1) {
          const source = resolved[index]();
          if (source != null && key in source) {
            return (source as Record<string | symbol, unknown>)[key];
          }
        }
        return undefined;
      },
      has(_target, key) {
        return resolved.some((read) => {
          const source = read();
          return source != null && key in source;
        });
      },
      ownKeys() {
        const seen = new Set<string | symbol>();
        for (const read of resolved) {
          const source = read();
          if (source != null) {
            for (const key of Reflect.ownKeys(source)) seen.add(key);
          }
        }
        return [...seen];
      },
      getOwnPropertyDescriptor(_target, key) {
        for (let index = resolved.length - 1; index >= 0; index -= 1) {
          const source = resolved[index]();
          if (source != null && key in source) {
            return { configurable: true, enumerable: true, writable: true, value: source[key as string] };
          }
        }
        return undefined;
      },
    },
  );
}

