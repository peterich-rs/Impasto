import { View as HostView, Stack as HostStack } from "@impasto/solid-renderer/host";
import {
  For,
  Show,
  createMemo,
  createSignal,
  createTrackedEffect,
  onCleanup,
  untrack,
  useContext,
  type Accessor,
} from "solid-js";
import { Errored } from "solid-js";
import {
  Text,
  scene,
  SubtreeVisibleContext,
  restoreFocus,
  spring,
  type NodeHandle,
} from "@impasto/solid-renderer";
import {
  PageActiveContext,
  PageKeepContext,
  type RouteEntry,
  type LazyRouteLoader,
  type RouteComponent,
  type RoutePhase,
  type Router,
} from "./stack.ts";
import { stackMotion, travelDistance } from "./transition.ts";
export type TransitionProps = {
  phase: RoutePhase;
  /** Capture when starting the motion, then call it when that motion ends. */
  onDone: () => void;
  children?: NodeHandle;
};
export type RouterViewProps = {
  router: Router;
  /** Transition selected when each route mounts. Defaults to the stack slide. */
  transition?: (props: TransitionProps) => NodeHandle;
  /** Page color for the default transition. Omitted leaves the background transparent. */
  background?: string;
  /** Slide fully opaque pages without blending either page during the transition. */
  opaque?: boolean;
  /** Default is the package's plain error page. */
  errorFallback?: (error: unknown, reset: () => void) => NodeHandle;
};

type LazyPage = { page?: RouteComponent; loading: Promise<RouteComponent> };
const lazyPages = new WeakMap<LazyRouteLoader, LazyPage>();

function LazyPageContent(props: {
  load: LazyRouteLoader;
  fallback?: () => NodeHandle;
  params: RouteEntry["params"];
  phase: RoutePhase;
}): unknown {
  let entry = lazyPages.get(props.load);
  if (!entry) {
    const load = props.load;
    const record: LazyPage = { loading: Promise.resolve().then(load).then(
      module => {
        if (typeof module.default !== "function") throw new TypeError("lazy route requires a default component");
        record.page = module.default;
        return module.default;
      },
    ) };
    void record.loading.catch(() => { if (lazyPages.get(load) === record) lazyPages.delete(load); });
    lazyPages.set(load, record);
    entry = record;
  }
  const [page, setPage] = createSignal<RouteComponent | undefined>(entry.page);
  const [failure, setFailure] = createSignal<{ error: unknown } | undefined>();
  if (!entry.page) {
    let active = true;
    onCleanup(() => { active = false; });
    void entry.loading.then(
      page => { if (active) setPage(() => page); },
      error => { if (active) setFailure({ error }); },
    );
  }
  return () => {
    const failed = failure();
    if (failed) throw failed.error;
    const Page = page();
    return Page ? <Page params={props.params} phase={props.phase} /> : props.fallback?.();
  };
}
function pageDistance(mover: number): number {
  const measured = scene.rect(mover).w;
  const width = scene.metrics?.().width ?? 0;
  return travelDistance(measured, width);
}
function plainError(error: unknown, reset: () => void): NodeHandle {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <HostView
      style={{
        gap: 8,
      }}
    >
      <Text>{message}</Text>
      <HostView onClick={() => reset()}>
        <Text>retry</Text>
      </HostView>
    </HostView>
  );
}
function unwrapError(error: unknown): unknown {
  return typeof error === "function" ? (error as () => unknown)() : error;
}
function loggedError(error: unknown): string {
  if (!(error instanceof Error)) {
    return String(error);
  }
  const stack = typeof error.stack === "string" ? error.stack : "";
  if (stack.length === 0) {
    return error.message;
  }
  return stack.includes(error.message) ? stack : `${error.message}\n${stack}`;
}
export function StackTransition(
  props: TransitionProps & {
    background?: string;
    opaque?: boolean;
  },
): NodeHandle {
  let mover = 0;
  let fade = 0;
  let played: RoutePhase | undefined;
  let completion: (() => void) | undefined;
  let pending = false;
  const cancellations: Array<() => void> = [];
  const cancelMotion = () => {
    for (const cancel of cancellations.splice(0)) cancel();
  };
  onCleanup(cancelMotion);
  const [mounted, setMounted] = createSignal(0);
  createTrackedEffect(() => {
    const phase = props.phase;
    const complete = props.onDone;
    if (mounted() === 0 || mover === 0 || fade === 0) {
      return;
    }
    if (phase === played && complete === completion) return;
    const reveal = phase === "live" && played === "cover" && pending;
    const retarget = phase !== "enter" && played !== undefined;
    played = phase;
    completion = complete;
    cancelMotion();
    const steps = (phase === "live"
      ? [{ prop: "translateX", from: 0, to: 0 }, { prop: "opacity", from: 255, to: 255 }]
      : stackMotion(phase, pageDistance(mover)))
      .filter((step) => !props.opaque || step.prop !== "opacity");
    let left = steps.length;
    pending = phase !== "live";
    for (const step of steps) {
      const isOpacity = step.prop === "opacity";
      const id = isOpacity ? fade : mover;
      const write = (value: number) => {
        scene.patchStyle(id, isOpacity ? { opacity: Math.round(value) / 255 } : { translateX: value });
      };
      if (phase === "live" && !reveal) {
        write(step.to);
        continue;
      }
      if (!retarget) write(step.from);
      cancellations.push(spring({
        id,
        ...step,
        write,
        retarget,
        onDone: phase === "live" ? undefined : () => {
          left -= 1;
          if (left === 0) {
            pending = false;
            complete();
          }
        },
      }));
    }
  });
  const body = (
    <HostView
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      {props.children}
    </HostView>
  );
  return (
    <HostStack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
        scene.setTransitionSnapshot?.(node, true);
        fade = node;
        setMounted(node);
      }}
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
        opacity: 1,
      }}
    >
      <HostStack
        ref={(node) => {
          if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
        scene.setTransitionSnapshot?.(node, true);
        mover = node;
        }}
        style={{
          width: "100%",
          height: "100%",
          flex: 1,
          flexShrink: 1,
          minWidth: 0,
          minHeight: 0,
        }}
      >
        {props.background === undefined ? (
          body
        ) : (
          <HostView
            style={{
              width: "100%",
              height: "100%",
              flex: 1,
              flexShrink: 1,
              minWidth: 0,
              minHeight: 0,
              backgroundColor: props.background,
            }}
          >
            {body}
          </HostView>
        )}
      </HostStack>
    </HostStack>
  );
}
function RouteHost(props: {
  router: Router;
  entry: Accessor<RouteEntry>;
  transition?: RouterViewProps["transition"];
  background?: string;
  opaque?: boolean;
  errorFallback?: RouterViewProps["errorFallback"];
}): NodeHandle {
  let previous = 0;
  let scope: NodeHandle | undefined;
  let lastFocused = 0;
  const parentVisible = useContext(SubtreeVisibleContext);
  const branchVisible = createMemo(() => parentVisible() && props.router.stack().some(entry => entry.key === props.entry().key));
  const showsBackScrim = () => {
    const preview = props.router.backPreview();
    if (!preview || scene.settings?.().reduceMotion) return false;
    const stack = props.router.stack();
    const index = stack.findIndex((entry) => entry.key === preview.key);
    return index > 0 && stack[index - 1]?.key === props.entry().key;
  };
  const active = (): boolean => {
    const list = props.router.stack();
    for (let index = list.length - 1; index >= 0; index -= 1) {
      const entry = list[index];
      if (entry && entry.phase !== "exit" && entry.phase !== "cover") {
        return entry.key === props.entry().key;
      }
    }
    return false;
  };
  onCleanup(() => {
    if (scope !== undefined && scene.popFocusScope?.(scope)) restoreFocus(previous);
  });
  const { component: Page, lazy, fallback, keepAlive } = untrack(() => props.entry());
  const params = createMemo(() => props.entry().params);
  const transitionEnd = createMemo((previous?: { entry: RouteEntry; done: () => void }) => {
    const entry = props.entry();
    return previous?.entry === entry ? previous : { entry, done: () => props.router.complete(entry) };
  });
  const transition =
    untrack(() => props.transition) ??
    ((step: TransitionProps) => (
      <StackTransition
        phase={step.phase}
        onDone={step.onDone}
        background={props.background}
        opaque={props.opaque}
      >
        {step.children}
      </StackTransition>
    ));
  return (
    <SubtreeVisibleContext value={branchVisible}>
    <PageActiveContext value={active}>
      <PageKeepContext value={() => keepAlive}>
        <HostStack
          ref={(node) => {
            if (node === null) return;
            scene.setComponentStyle(node, {
              gridTemplateColumns: "minmax(0, 1fr)",
              gridTemplateRows: "minmax(0, 1fr)",
            });
            createTrackedEffect(() => {
              if (!branchVisible()) {
                if (scope !== undefined) {
                  if (scene.focused?.()) lastFocused = scene.focused();
                  if (scene.popFocusScope?.(scope)) restoreFocus(previous);
                  scope = undefined;
                }
                scene.patchStyle(node, { display: "none" });
              } else {
                scene.clearStyle(node, ["display"]);
                if (scope === undefined) {
                  previous = scene.focused?.() ?? 0;
                  try {
                    scene.pushFocusScope(node);
                    scope = node;
                    if (lastFocused) restoreFocus(lastFocused);
                  } catch {
                    // Mock hosts can omit the scope stack.
                  }
                }
              }
              scene.setSemanticsHidden?.(node, !active());
            });
            let previewing = false;
            createTrackedEffect(() => {
              const preview = props.router.backPreview();
              if (preview?.key === props.entry().key && !scene.settings?.().reduceMotion) {
                const width = scene.metrics?.().width ?? 0;
                scene.patchStyle(node, { translateX: width * preview.progress * (preview.fromRight ? -0.25 : 0.25) });
                previewing = true;
              } else if (previewing) {
                scene.clearStyle(node, ["translateX"]);
                previewing = false;
              }
            });
          }}
          style={{
            width: "100%",
            height: "100%",
            flex: 1,
            flexShrink: 1,
            minWidth: 0,
            minHeight: 0,
          }}
        >
          {/* The entering page can receive taps while the exposed lower page is blocked. */}
          <Show when={props.entry().phase === "enter"}>
            <HostView style={{ width: "100%", height: "100%" }} onClick={() => {}} />
          </Show>
          {transition({
            // The transition tracks its phase; the route's content keeps its owner.
            get phase() {
              return props.entry().phase;
            },
            get onDone() {
              return transitionEnd().done;
            },
            get children() {
              return (
                <Errored
                  fallback={(err, reset) => {
                    const error = unwrapError(err);
                    console.error(loggedError(error));
                    const fallback = props.errorFallback ?? plainError;
                    return fallback(error, () => reset());
                  }}
                >
                  {Page
                    ? <Page params={params()} phase={props.entry().phase} />
                    : lazy
                      ? <LazyPageContent load={lazy} fallback={fallback} params={params()} phase={props.entry().phase} />
                      : 0}
                </Errored>
              );
            },
          })}
          <Show when={showsBackScrim()}>
            <HostView
              ref={(node) => {
                if (node === null || typeof scene.bindBackProgressOpacity !== "function") return;
                const rule = scene.bindBackProgressOpacity(node, [0, 1], [0.24, 0]);
                onCleanup(() => scene.unbindScrollProp(rule));
              }}
              style={{
                position: "absolute", top: 0, left: 0, width: "100%", height: "100%",
                backgroundColor: "#000000",
              }}
            />
          </Show>
        </HostStack>
      </PageKeepContext>
    </PageActiveContext>
    </SubtreeVisibleContext>
  );
}
export function RouterView(props: RouterViewProps): NodeHandle {
  return (
    <HostStack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
      }}
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
        overflow: "clip",
      }}
    >
      <For each={props.router.retainedStack?.() ?? props.router.stack()} keyed={(entry) => entry.key}>
        {(entry) => (
          <RouteHost
            router={props.router}
            entry={entry}
            transition={props.transition}
            background={props.background}
            opaque={props.opaque}
            errorFallback={props.errorFallback}
          />
        )}
      </For>
    </HostStack>
  );
}
