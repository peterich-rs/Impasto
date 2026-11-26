import {
  createContext,
  createTrackedEffect,
  createSignal,
  getOwner,
  onCleanup,
  useContext,
  type Accessor,
} from "solid-js";
import { onBack, onFrame, type NodeHandle } from "@impasto/solid-renderer";
import { matchPath, routePath } from "./path.ts";
export type RouteParams = Record<string, string>;
export type RouteComponent = (props: { params: RouteParams; phase: RoutePhase }) => NodeHandle;
export type LazyRouteLoader = () => Promise<{ default: RouteComponent }>;
export type RouteDef = {
  path: string;
  fallback?: () => NodeHandle;
  keepAlive?: boolean;
} & ({ component: RouteComponent; lazy?: never } | { lazy: LazyRouteLoader; component?: never });
export type RoutePhase = "enter" | "live" | "exit" | "cover";
export type RouteEntry = {
  key: string;
  path: string;
  pattern: string;
  params: RouteParams;
  phase: RoutePhase;
  keepAlive: boolean;
  component: RouteDef["component"];
  lazy?: LazyRouteLoader;
  fallback?: RouteDef["fallback"];
};
export type Router = {
  stack: Accessor<RouteEntry[]>;
  depth: Accessor<number>;
  push: (path: string) => boolean;
  replace: (path: string) => boolean;
  pop: () => boolean;
  popTo: (path: string) => boolean;
  reset: (path: string) => boolean;
  complete: (entry: RouteEntry) => void;
  backPreview: Accessor<{ key: string; progress: number; fromRight: boolean } | null>;
};
type RouteRestoration = {
  key: string;
  get(key: string): string | null;
  set(key: string, value: string | null): void;
};
export const PageActiveContext = createContext<Accessor<boolean>>(() => true);
export const PageKeepContext = createContext<Accessor<boolean>>(() => false);

/**
 * keepAlive pages do not register `onFrame` while they are not the focused route.
 * Other pages keep their listener.
 */
export function usePageFrame(
  fn: (targetMs: number, intervalMs: number) => void,
): void {
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
function resolved(
  defs: readonly RouteDef[],
  path: string,
): Omit<RouteEntry, "key" | "phase"> | null {
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
        lazy: def.lazy,
        fallback: def.fallback,
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

function restoreRoutes(defs: readonly RouteDef[], restoration: RouteRestoration): RouteEntry[] | null {
  try {
    const saved = restoration.get(restoration.key);
    if (!saved) return null;
    const value: unknown = JSON.parse(saved);
    if (!Array.isArray(value) || value[0] !== 1 || value.length < 2) return null;
    const entries: RouteEntry[] = [];
    for (const path of value.slice(1)) {
      const entry = typeof path === "string" && resolved(defs, path);
      if (!entry) return null;
      entries.push({ ...entry, key: nextKey(), phase: "live" });
    }
    return entries;
  } catch {
    return null;
  }
}
export function createRouter(
  defs: readonly RouteDef[],
  options: {
    initial?: string;
    onBack?: () => boolean;
    restoration?: RouteRestoration;
    /** Disable the automatic link handler when the application restores or maps its own routes. */
    linking?: boolean;
  } = {},
): Router {
  const initialPath = options.initial ?? "/";
  const first = resolved(defs, initialPath);
  const restored = options.restoration && restoreRoutes(defs, options.restoration);
  const [entries, setEntries] = createSignal<RouteEntry[]>(
    restored ?? (first
      ? [
          {
            ...first,
            key: nextKey(),
            phase: "live",
          },
        ]
      : []),
  );
  const [backPreview, setBackPreview] = createSignal<ReturnType<Router["backPreview"]>>(null);
  function navigate(path: string, previousPhase: "cover" | "exit"): boolean {
    const match = resolved(defs, path);
    if (!match) {
      return false;
    }
    let accepted = false;
    setEntries((list) => {
      // A superseded exit can be unmounted. It is already out of navigation history.
      const history = list.filter((entry) => entry.phase !== "exit");
      const top = history.pop();
      if (previousPhase === "cover" && top?.phase === "enter" && top.path === match.path) {
        return list;
      }
      accepted = true;
      if (top) history.push({ ...top, phase: previousPhase });
      history.push({ ...match, key: nextKey(), phase: top ? "enter" : "live" });
      return history;
    });
    return accepted;
  }
  function pop(): boolean {
    let handled = false;
    setEntries((list) => {
      if (list.at(-1)?.phase === "exit") {
        handled = true;
        return list;
      }
      const history = list.filter((entry) => entry.phase !== "exit");
      const top = history.pop();
      const revealed = history.at(-1);
      if (!top || !revealed) return list;
      handled = true;
      if (revealed.phase !== "live") {
        history[history.length - 1] = { ...revealed, phase: "live" };
      }
      history.push({ ...top, phase: "exit" });
      return history;
    });
    return handled;
  }
  function popTo(path: string): boolean {
    const pathname = routePath(path);
    let found = false;
    setEntries((list) => {
      const history = list.filter((entry) => entry.phase !== "exit");
      let index = history.length - 1;
      while (index >= 0 && history[index]?.path !== pathname) index -= 1;
      if (index < 0 || index === history.length - 1) {
        return list;
      }
      found = true;
      return history.map((entry, cursor) => {
        if (cursor > index) return { ...entry, phase: "exit" };
        if (cursor === index && entry.phase !== "live") return { ...entry, phase: "live" };
        return entry;
      });
    });
    return found;
  }
  function reset(path: string): boolean {
    const match = resolved(defs, path);
    if (!match) {
      return false;
    }
    setEntries([
      {
        ...match,
        key: nextKey(),
        phase: "live",
      },
    ]);
    return true;
  }
  function complete(entry: RouteEntry): void {
    if (entry.phase === "live") return;
    setEntries((list) => {
      // An entry's key survives a phase change. Its object identifies the motion that started.
      const index = list.indexOf(entry);
      if (index < 0) return list;
      const next = list.slice();
      if (entry.phase === "exit") next.splice(index, 1);
      else next[index] = { ...entry, phase: "live" };
      return next;
    });
  }
  const router: Router = {
    stack: entries,
    depth: () => entries().length,
    push: (path) => navigate(path, "cover"),
    replace: (path) => navigate(path, "exit"),
    pop,
    popTo,
    reset,
    complete,
    backPreview,
  };
  const activeRoutes = () => entries().filter((entry) => entry.phase !== "exit");
  if (options.restoration) {
    const restoration = options.restoration;
    let last: string | undefined;
    createTrackedEffect(() => {
      const value = JSON.stringify([1, ...activeRoutes().map((entry) => entry.path)]);
      if (last === value) return;
      last = value;
      try {
        restoration.set(restoration.key, value);
      } catch (error) {
        console.warn("无法保存路由状态", error);
      }
    });
  }
  let committedKey: string | undefined;
  const releasePop = onBack(200, () => {
    const key = committedKey;
    committedKey = undefined;
    if (key && key !== activeRoutes().at(-1)?.key) return true;
    return pop();
  }, {
    enabled: () => activeRoutes().length > 1 || entries().at(-1)?.phase === "exit",
    gesture: (event) => {
      if (event.phase === "start") {
        const key = activeRoutes().at(-1)?.key;
        if (key) setBackPreview({ key, fromRight: event.fromRight, progress: 0 });
      } else if (event.phase === "progress") {
        setBackPreview((preview) => preview && { ...preview, progress: event.progress });
      } else {
        committedKey = event.phase === "commit" ? backPreview()?.key : undefined;
        setBackPreview(null);
      }
    },
  });
  const releaseApp = options.onBack ? onBack(100, options.onBack) : () => {};
  let disposed = false;
  let releaseLinking: (() => void) | undefined;
  if (getOwner()) {
    onCleanup(() => {
      disposed = true;
      releasePop();
      releaseApp();
      releaseLinking?.();
    });
  }
  if (options.linking !== false) {
    void import("@impasto/linking")
      .then(({ Linking }) => {
        if (disposed) return;
        const initial = Linking.getInitialURL();
        if (initial) reset(initial);
        const navigate = ({ url }: { url: string }) => { router.push(url); };
        Linking.addEventListener("url", navigate);
        releaseLinking = () => Linking.removeEventListener("url", navigate);
      })
      .catch(() => {
        /* This host has no linking capability. */
      });
  }
  return router;
}
