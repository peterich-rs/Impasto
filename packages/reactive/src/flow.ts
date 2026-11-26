// Control flow stays in JS. It calls the native memos and effects; it does not
// schedule anything itself. Solid 2.0.0 is the reference for these components.

import { children, createMemo, createRoot, createSignal, getOwner, onCleanup, runWithOwner, untrack } from "./native.ts";
import type { JSX } from "./types.ts";

type Child = JSX.Element | boolean | string | null | undefined | readonly Child[];

type MaybeAccessor<T> = T | (() => T);

function read<T>(value: MaybeAccessor<T>): T {
  return typeof value === "function" ? (value as () => T)() : value;
}

export function Show<T>(props: { when: T | undefined | null | false; keyed: true; fallback?: Child; children: Child | ((item: T) => Child) }): JSX.Element;
export function Show<T>(props: { when: T | undefined | null | false; keyed?: false; fallback?: Child; children: Child | ((item: () => T) => Child) }): JSX.Element;
export function Show<T>(props: {
  when: T | undefined | null | false;
  keyed?: boolean;
  fallback?: Child;
  children: Child | ((item: T) => Child) | ((item: () => T) => Child);
}): JSX.Element {
  const keyed = props.keyed;
  const condition = createMemo(() => props.when);
  const truthy = keyed ? condition : createMemo(() => Boolean(condition()));
  let shown = false;
  return createMemo(() => {
    const on = truthy();
    const value = on;
    shown = Boolean(on);
    if (!on) {
      return props.fallback;
    }
    const child = props.children;
    const fn = typeof child === "function" && child.length > 0;
    if (!fn) {
      return child as JSX.Element;
    }
    if (keyed) {
      return untrack(() => (child as (item: T) => JSX.Element)(value as T));
    }
    return untrack(() =>
      (child as (item: () => T) => JSX.Element)(() => {
        if (!shown) {
          throw new Error("Stale read from <Show>.");
        }
        return condition() as T;
      }),
    );
  }) as unknown as JSX.Element;
}

export function Match<T>(props: {
  when: T | undefined | null | false;
  keyed?: boolean;
  children: Child | ((item: T) => Child) | ((item: () => T) => Child);
}): JSX.Element {
  return props as unknown as JSX.Element;
}

export function Switch(props: {
  fallback?: Child;
  children: JSX.Element;
}): JSX.Element {
  const list = children(() => props.children);
  const matches = createMemo(() => list.toArray().filter(Boolean).map(item => {
    const match = item as { when: unknown; keyed?: boolean; children: unknown };
    const value = createMemo(() => match.when);
    const active = match.keyed ? value : createMemo(() => Boolean(value()));
    return { match, value, active };
  }));
  const select = createMemo(() => {
    for (const entry of matches()) {
      const value = entry.active();
      if (value) return { entry, value };
    }
    return undefined;
  });
  return createMemo(() => {
    const chosen = select();
    if (!chosen) return props.fallback;
    const { entry, value } = chosen;
    const child = entry.match.children;
    if (typeof child !== "function" || child.length === 0) return child;
    if (entry.match.keyed) return untrack(() => child(value));
    return untrack(() => child(() => {
      if (untrack(select)?.entry !== entry) throw new Error("Stale read from <Match>.");
      return entry.value();
    }));
  }) as unknown as JSX.Element;
}

export function For<T>(props: { each: MaybeAccessor<readonly T[] | undefined | null>; keyed?: true; fallback?: Child; children: (item: T, index: () => number) => Child }): JSX.Element;
export function For<T>(props: { each: MaybeAccessor<readonly T[] | undefined | null>; keyed: false; fallback?: Child; children: (item: () => T, index: number) => Child }): JSX.Element;
export function For<T>(props: { each: MaybeAccessor<readonly T[] | undefined | null>; keyed: (item: T) => unknown; fallback?: Child; children: (item: () => T, index: () => number) => Child }): JSX.Element;
export function For<T>(props: ForProps<T>): JSX.Element {
  return createMemo(mapFor(props)) as unknown as JSX.Element;
}

type ForProps<T> = {
  each: MaybeAccessor<readonly T[] | undefined | null>;
  keyed?: boolean | ((item: T) => unknown);
  fallback?: Child;
  children: Function;
};

export function mapFor<T>(props: ForProps<T>): () => Child {
  const owner = getOwner();
  type Row = { key: unknown; item: T; position: number; value: Child; write: (value: () => T) => void; index: (index: number) => void; dispose: () => void };
  const negativeZero = Symbol();
  const mapKey = (key: unknown) => Object.is(key, -0) ? negativeZero : key;
  let rows: Row[] = [];
  onCleanup(() => { for (const row of rows) row.dispose(); rows = []; });
  return () => {
    const values = read(props.each) ?? [];
    const remaining = new Map<unknown, { rows: Row[]; next: number }>();
    const retained = new Set<Row>();
    for (const row of rows) {
      const key = mapKey(row.key);
      const bucket = remaining.get(key);
      if (bucket) bucket.rows.push(row);
      else remaining.set(key, { rows: [row], next: 0 });
    }
    const keyed = props.keyed;
    const next: Row[] = [];
    for (let index = 0; index < values.length; index++) {
      const item = values[index];
      const key = keyed === false ? index : typeof keyed === "function" ? keyed(item) : item;
      const bucket = remaining.get(mapKey(key));
      let row = bucket && bucket.next < bucket.rows.length ? bucket.rows[bucket.next++] : undefined;
      if (!row) {
        row = runWithOwner(owner, () => createRoot(dispose => {
          const [state, writeItem] = createSignal({ value:item });
          const readItem = () => state().value;
          const write = (next:()=>T) => { writeItem({value:next()}); };
          const [readIndex, writeIndex] = createSignal(index);
          const accessor = keyed === false || typeof keyed === "function";
          const value = untrack(() => props.children(accessor ? readItem : item, keyed === false ? index : readIndex));
          return { key, item, position: index, value, write, index: writeIndex, dispose };
        }));
      } else {
        retained.add(row);
        if (!Object.is(row.item, item)) {
          row.item = item;
          row.write(() => item);
        }
        if (row.position !== index) {
          row.position = index;
          row.index(index);
        }
      }
      next.push(row);
    }
    for (const row of rows) if (!retained.has(row)) row.dispose();
    rows = next;
    return rows.length ? rows.map(row => row.value) : props.fallback;
  };
}

export function Errored(props: {
  fallback: JSX.Element | ((error: unknown, reset: () => void) => JSX.Element);
  children: JSX.Element;
}): JSX.Element {
  const [error, setError] = createSignal<unknown>();
  const reset = () => setError(undefined);
  return createMemo(() => {
    const failure = error();
    if (failure !== undefined) return typeof props.fallback === "function" ? props.fallback(failure, reset) : props.fallback;
    return createRoot(() => catchError(() => children(() => props.children), failure => setError(() => failure)));
  }) as unknown as JSX.Element;
}
import { catchError } from "./native.ts";
