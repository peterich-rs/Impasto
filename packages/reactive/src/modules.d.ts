declare module "reactive" {
  export {
    createSignal, createMemo, createRenderEffect, createTrackedEffect, createRoot,
    getOwner, runWithOwner, onCleanup, untrack, createContext, useContext, children,
  } from "solid-js";
  export function catchError<T>(run: () => T, handler: (error: unknown) => void): T;
  export function batch<T>(scope: () => T): T;
}
declare module "virtual:impasto-locales" {
  export const defaultLocale: string;
  export const messages: Record<string, Record<string, string | { one: string; other: string; zero?: string }>>;
}
declare module "scene" {
  type Node = import("../../solid-renderer/src/tree.ts").Node;
  export const scene: import("../../solid-renderer/src/scene.ts").SceneModule;
  export function createElement(tag: string, props?: Record<string, unknown>): Node;
  export function createTextNode(value: string | number): Node;
  export function insert(parent: Node, children: unknown, marker?: Node | null): void;
  export function insertNode(parent: Node, child: Node, anchor?: Node): void;
  export function defineStyle(input: unknown): number;
  export function defineTemplate(input: unknown): number;
  type LinkedTemplate = readonly unknown[];
  export function instantiateTemplate(id: number | LinkedTemplate): Node;
  export function instantiateTemplate(id: number | LinkedTemplate, withBindings: true): Node[];
  export function applyStaticStyle(node: Node, id: number | readonly unknown[]): void;
  export function setProp(node: Node, name: string, value: unknown, previous?: unknown): void;
  export function patchInlineStyle(node: Node, selector: string | readonly string[], value: unknown): void;
  export function patchScalarStyle(node: Node, op: number, value: unknown): void;
  export function applyStaticClass(node: Node, id: number): void;
  export function applyStaticClassList(node: Node, ids: readonly (number | undefined)[]): void;
  export { createRenderEffect as effect, createMemo as memo } from "reactive";
  export function createComponent<P, T>(component: (props: P) => T, props: P): T;
  export const Path2D: import("../../solid-renderer/src/canvas/types.ts").Path2DConstructor;
  export function bindCanvas(
    node: import("../../solid-renderer/src/tree.ts").Node,
    draw: (
      context: import("../../solid-renderer/src/canvas/types.ts").CanvasRenderingContext2D,
      size: import("../../solid-renderer/src/canvas/types.ts").CanvasSize,
    ) => unknown,
    onDrawError?: (
      error: import("../../solid-renderer/src/canvas/types.ts").CanvasError,
    ) => void,
    deferWhenOffscreen?: boolean,
  ): import("../../solid-renderer/src/canvas/types.ts").CanvasBinding;
}
