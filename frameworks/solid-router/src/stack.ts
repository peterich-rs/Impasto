import { createContext, createTrackedEffect, createSignal, getOwner, onCleanup, useContext, type Accessor } from "solid-js";
import { onBack, onFrame, type NodeHandle } from "@uifw/solid-renderer";
import type { Linking } from "@uifw/modules";
import { matchPath, routePath } from "./path.ts";

export type RouteParams = Record<string, string>;

export type RouteDef = {
  path: string;
  component: (props: { params: RouteParams }) => NodeHandle;
  keepAlive?: boolean;
};

export type RoutePhase = "enter" | "live" | "exit" | "cover";

export type RouteEntry = {
  key: string;
  path: string;
  pattern: string;
  params: RouteParams;
  phase: RoutePhase;
  keepAlive: boolean;
  component: RouteDef["component"];
};

export type Router = {
  stack: Accessor<RouteEntry[]>;
  depth: Accessor<number>;
  push: (path: string) => boolean;
  replace: (path: string) => boolean;
  pop: () => boolean;
  popTo: (path: string) => boolean;
  reset: (path: string) => boolean;
  complete: (key: string) => void;
};

export const PageActiveContext = createContext<Accessor<boolean>>(() => true);
export const PageKeepContext = createContext<Accessor<boolean>>(() => false);

/**
 * keepAlive pages do not register `onFrame` while they are not the focused route.
 * Other pages keep their listener.
 */
export function usePageFrame(fn: (targetMs: number, intervalMs: number) => void): void {
  const active = useContext(PageActiveContext);
  const keepAlive = useContext(PageKeepContext);
  createTrackedEffect(() => {
    if (keepAlive() && !active()) {
      return;
    }
    const stop = onFrame(fn);
    onCleanup(stop);
  });
}

function resolved(defs: readonly RouteDef[], path: string): Omit<RouteEntry, "key" | "phase"> | null {
  const pathname = routePath(path);
  for (const def of defs) {
    const params = matchPath(def.path, pathname);
    if (params) {
      return {
        path: pathname,
        pattern: def.path,
        params,
        keepAlive: !!def.keepAlive,
        component: def.component,
      };
    }
  }
  return null;
}

let keySeq = 1;
function nextKey(): string {
  const key = `route-${keySeq}`;
  keySeq += 1;
  return key;
}

export function createRouter(
  defs: readonly RouteDef[],
  options: { initial?: string; onBack?: () => boolean } = {},
): Router {
  const initialPath = options.initial ?? "/";
  const first = resolved(defs, initialPath);
  const [entries, setEntries] = createSignal<RouteEntry[]>(
    first ? [{ ...first, key: nextKey(), phase: "live" }] : [],
  );

  function push(path: string): boolean {
    const match = resolved(defs, path);
    if (!match) {
      return false;
    }
    const list = entries();
    const next = { ...match, key: nextKey(), phase: "enter" as const };
    const top = list[list.length - 1];
    if (!top) {
      setEntries([{ ...next, phase: "live" }]);
      return true;
    }
    setEntries([...list.slice(0, -1), { ...top, phase: "cover" }, next]);
    return true;
  }

  function replace(path: string): boolean {
    const match = resolved(defs, path);
    if (!match) {
      return false;
    }
    const list = entries();
    const top = list[list.length - 1];
    const next = { ...match, key: nextKey(), phase: "enter" as const };
    if (!top) {
      setEntries([{ ...next, phase: "live" }]);
      return true;
    }
    setEntries([...list.slice(0, -1), { ...top, phase: "exit" }, next]);
    return true;
  }

  function pop(): boolean {
    const list = entries();
    if (list.length <= 1) {
      return false;
    }
    const top = list[list.length - 1];
    if (!top) {
      return false;
    }
    if (top.phase === "exit") {
      return true;
    }
    setEntries(
      list.map((entry, index) => {
        if (index === list.length - 1) {
          return { ...entry, phase: "exit" as const };
        }
        if (entry.phase === "cover") {
          return { ...entry, phase: "live" as const };
        }
        return entry;
      }),
    );
    return true;
  }

  function popTo(path: string): boolean {
    const pathname = routePath(path);
    const list = entries();
    let index = -1;
    for (let cursor = list.length - 1; cursor >= 0; cursor -= 1) {
      if (list[cursor]?.path === pathname && list[cursor]?.phase !== "exit") {
        index = cursor;
        break;
      }
    }
    if (index < 0) {
      return false;
    }
    if (index === list.length - 1) {
      return false;
    }
    setEntries(list.map((entry, cursor) => (cursor > index ? { ...entry, phase: "exit" } : entry)));
    return true;
  }

  function reset(path: string): boolean {
    const match = resolved(defs, path);
    if (!match) {
      return false;
    }
    setEntries([{ ...match, key: nextKey(), phase: "live" }]);
    return true;
  }

  function complete(key: string): void {
    setEntries(
      entries().flatMap((entry) => {
        if (entry.key !== key) {
          return [entry];
        }
        if (entry.phase === "exit") {
          return [];
        }
        if (entry.phase === "enter" || entry.phase === "cover") {
          return [{ ...entry, phase: "live" as const }];
        }
        return [entry];
      }),
    );
  }

  const router: Router = {
    stack: entries,
    depth: () => entries().length,
    push,
    replace,
    pop,
    popTo,
    reset,
    complete,
  };

  const releasePop = onBack(200, () => pop());
  const releaseApp = options.onBack ? onBack(100, options.onBack) : () => {};
  if (getOwner()) {
    onCleanup(() => {
      releasePop();
      releaseApp();
    });
  }

  const registry = (
    globalThis as { TurboModuleRegistry?: { get(name: string): Linking | null } }
  ).TurboModuleRegistry;
  const linking = registry?.get("Linking") ?? null;
  if (linking) {
    const initialUrl = linking.getInitialURL();
    if (initialUrl) {
      reset(initialUrl);
    }
    linking.addEventListener("url", (event) => {
      push(event.url);
    });
  }

  return router;
}
