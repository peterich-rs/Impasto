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
export type RouteDef = {
  path: string;
  component: (props: { params: RouteParams; phase: RoutePhase }) => NodeHandle;
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
  complete: (entry: RouteEntry) => void;
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
  options: {
    initial?: string;
    onBack?: () => boolean;
  } = {},
): Router {
  const initialPath = options.initial ?? "/";
  const first = resolved(defs, initialPath);
  const [entries, setEntries] = createSignal<RouteEntry[]>(
    first
      ? [
          {
            ...first,
            key: nextKey(),
            phase: "live",
          },
        ]
      : [],
  );
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
  };
  const releasePop = onBack(200, () => pop());
  const releaseApp = options.onBack ? onBack(100, options.onBack) : () => {};
  if (getOwner()) {
    onCleanup(() => {
      releasePop();
      releaseApp();
    });
  }
  void import("@impasto/linking")
    .then(({ Linking }) => {
      const initial = Linking.getInitialURL();
      if (initial) reset(initial);
      Linking.addEventListener("url", (event) => router.push(event.url));
    })
    .catch(() => {
      /* unregistered: same as today's get() == null */
    });
  return router;
}
