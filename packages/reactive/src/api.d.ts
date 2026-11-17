export type { JSX } from "../../solid-renderer/src/jsx.ts";
export type Accessor<T> = () => T;
export type Setter<T> = (next: T | ((previous: T) => T)) => T;
export type Owner = number;
export function createSignal<T>(value: T): [Accessor<T>, Setter<T>];
export function createSignal<T>(): [Accessor<T | undefined>, Setter<T | undefined>];
export function createMemo<T>(compute: (previous?: T) => T): Accessor<T>;
export function createRenderEffect<T>(compute: (previous?: T) => T, effect: (value: T, previous?: T) => void | (() => void), options?: { schedule?: boolean; sync?: boolean; transparent?: boolean }): void;
export function createTrackedEffect(compute: () => void | (() => void)): void;
export function createRoot<T>(run: (dispose: () => void) => T): T;
export function getOwner(): Owner | null;
export function runWithOwner<T>(owner: Owner | null, run: () => T): T;
export function onCleanup(cleanup: () => void): void;
export function untrack<T>(read: () => T): T;
export function batch<T>(run: () => T): T;
export function children(read: () => unknown): (() => unknown) & { toArray(): unknown[] };
export interface Context<T> {
  (props: { value: T; children?: unknown }): import("../../solid-renderer/src/tree.ts").Node;
  Provider: Context<T>;
  defaultValue: T;
  id: symbol;
}
export function createContext<T>(defaultValue: T): Context<T>;
export function createContext<T>(): Context<T | undefined>;
export function useContext<T>(context: Context<T>): T;
export { Show, Switch, Match, For, Errored } from "./flow.ts";
