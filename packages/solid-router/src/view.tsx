import {
  For,
  Show,
  createMemo,
  createSignal,
  createTrackedEffect,
  onCleanup,
  untrack,
  type Accessor,
} from "solid-js";
import { Errored } from "solid-js";
import {
  Text,
  scene,
  restoreFocus,
  spring,
  type NodeHandle,
} from "@impasto/solid-renderer";
import {
  PageActiveContext,
  PageKeepContext,
  type RouteEntry,
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
  /** Default stack slide, including clip, pointer, and pageDistance. */
  transition?: (props: TransitionProps) => NodeHandle;
  /** Page color for the default transition. Omitted leaves the background transparent. */
  background?: string;
  /** Slide fully opaque pages without blending either page during the transition. */
  opaque?: boolean;
  /** Default is the package's plain error page. */
  errorFallback?: (error: unknown, reset: () => void) => NodeHandle;
};
function pageDistance(mover: number): number {
  const measured = scene.rect(mover).w;
  const width = scene.metrics?.().width ?? 0;
  return travelDistance(measured, width);
}
function plainError(error: unknown, reset: () => void): NodeHandle {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <view
      style={{
        gap: 8,
      }}
    >
      <Text>{message}</Text>
      <view onClick={() => reset()}>
        <Text>retry</Text>
      </view>
    </view>
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
    <view
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
    </view>
  );
  return (
    <stack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
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
      <stack
        ref={(node) => {
          if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
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
          <view
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
          </view>
        )}
      </stack>
    </stack>
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
  const { component: Page, keepAlive } = untrack(props.entry);
  const params = createMemo(() => props.entry().params);
  const transitionEnd = createMemo((previous?: { entry: RouteEntry; done: () => void }) => {
    const entry = props.entry();
    return previous?.entry === entry ? previous : { entry, done: () => props.router.complete(entry) };
  });
  const transition =
    props.transition ??
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
    <PageActiveContext value={active}>
      <PageKeepContext value={() => keepAlive}>
        <stack
          ref={(node) => {
            if (node === null) return;
            scene.setComponentStyle(node, {
              gridTemplateColumns: "minmax(0, 1fr)",
              gridTemplateRows: "minmax(0, 1fr)",
            });
            previous =
              typeof scene.focused === "function" ? scene.focused() : 0;
            try {
              scene.pushFocusScope(node);
              scope = node;
            } catch {
              // Mock hosts without a scope stack still mount the page.
            }
            createTrackedEffect(() => scene.setSemanticsHidden?.(node, !active()));
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
            <view style={{ width: "100%", height: "100%" }} onClick={() => {}} />
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
                  <Page params={params()} phase={props.entry().phase} />
                </Errored>
              );
            },
          })}
        </stack>
      </PageKeepContext>
    </PageActiveContext>
  );
}
export function RouterView(props: RouterViewProps): NodeHandle {
  return (
    <stack
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
      <For each={props.router.stack()} keyed={(entry) => entry.key}>
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
    </stack>
  );
}
