declare module "reactive" {
  export {
    createSignal, createMemo, createRenderEffect, createTrackedEffect, createRoot,
    getOwner, runWithOwner, onCleanup, untrack, createContext, useContext, children,
  } from "solid-js";
  export function catchError<T>(run: () => T, handler: (error: unknown) => void): T;
  export function batch<T>(scope: () => T): T;
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
  export function instantiateTemplate(id: number): Node;
  export function applyStaticStyle(node: Node, id: number): void;
  export function setProp(node: Node, name: string, value: unknown): void;
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
