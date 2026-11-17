// Control flow stays in JS. It calls the native memos and effects; it does not
// schedule anything itself. The shapes match solid-js 2.0.0-rc.17.

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
export function For<T>(props: {
  each: MaybeAccessor<readonly T[] | undefined | null>;
  keyed?: boolean | ((item: T) => unknown);
  fallback?: Child;
  children: Function;
}): JSX.Element {
  const owner = getOwner();
  type Row = { key: unknown; value: Child; write: (value: () => T) => void; index: (index: number) => void; dispose: () => void };
  let rows: Row[] = [];
  onCleanup(() => { for (const row of rows) row.dispose(); rows = []; });
  return createMemo(() => {
    const values = read(props.each) ?? [];
    const remaining = rows.slice();
    const next: Row[] = [];
    for (let index = 0; index < values.length; index++) {
      const item = values[index];
      const key = props.keyed === false ? index : typeof props.keyed === "function" ? props.keyed(item) : item;
      const at = remaining.findIndex(row => Object.is(row.key, key));
      let row = at < 0 ? undefined : remaining.splice(at, 1)[0];
      if (!row) {
        row = runWithOwner(owner, () => createRoot(dispose => {
          const [state, writeItem] = createSignal({ value:item });
          const readItem = () => state().value;
          const write = (next:()=>T) => { writeItem({value:next()}); };
          const [readIndex, writeIndex] = createSignal(index);
          const accessor = props.keyed === false || typeof props.keyed === "function";
          const value = untrack(() => props.children(accessor ? readItem : item, props.keyed === false ? index : readIndex));
          return { key, value, write, index: writeIndex, dispose };
        }));
      } else { row.write(() => item); row.index(index); }
      next.push(row);
    }
    for (const row of remaining) row.dispose();
    rows = next;
    return rows.length ? rows.map(row => row.value) : props.fallback;
  }) as unknown as JSX.Element;
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
