import {
  For,
  createSignal,
  createTrackedEffect,
  onCleanup,
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
  onDone: () => void;
  children?: NodeHandle;
};
export type RouterViewProps = {
  router: Router;
  /** Default stack slide, including clip, pointer, and pageDistance. */
  transition?: (props: TransitionProps) => NodeHandle;
  /** Page color for the default transition. Omitted leaves the background transparent. */
  background?: string;
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
  },
): NodeHandle {
  let mover = 0;
  let fade = 0;
  let played = "";
  const [mounted, setMounted] = createSignal(0);
  createTrackedEffect(() => {
    const phase = props.phase;
    if (mounted() === 0 || mover === 0 || fade === 0 || phase === played) {
      return;
    }
    played = phase;
    if (phase === "live") {
      scene.patchStyle(mover, {
        translateX: 0,
        translateY: 0,
        rotate: 0,
        scaleX: 1,
        scaleY: 1,
      });
      scene.patchStyle(fade, {
        opacity: 255 / 255,
      });
      return;
    }
    const steps = stackMotion(
      phase === "cover" ? "cover" : phase,
      pageDistance(mover),
    );
    let left = steps.length;
    for (const step of steps) {
      if (step.prop === "opacity") {
        scene.patchStyle(fade, {
          opacity: Math.round(step.from) / 255,
        });
      } else {
        scene.patchStyle(mover, {
          translateX: step.from,
          translateY: 0,
          rotate: 0,
          scaleX: 1,
          scaleY: 1,
        });
      }
      spring({
        id: step.prop === "opacity" ? fade : mover,
        prop: step.prop,
        from: step.from,
        to: step.to,
        write: (value) => {
          if (step.prop === "opacity") {
            scene.patchStyle(fade, {
              opacity: Math.round(value) / 255,
            });
          } else {
            scene.patchStyle(mover, {
              translateX: value,
              translateY: 0,
              rotate: 0,
              scaleX: 1,
              scaleY: 1,
            });
          }
        },
        onDone: () => {
          left -= 1;
          if (left <= 0) {
            props.onDone();
          }
        },
      });
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
  errorFallback?: RouterViewProps["errorFallback"];
}): NodeHandle {
  let previous = 0;
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
    try {
      scene.popFocusScope();
    } catch {
      // core owns the scope stack. This pop matches the ref's push.
    }
    restoreFocus(previous);
  });
  const Page = props.entry().component;
  const transition =
    props.transition ??
    ((step: TransitionProps) => (
      <StackTransition
        phase={step.phase}
        onDone={step.onDone}
        background={props.background}
      >
        {step.children}
      </StackTransition>
    ));
  return (
    <PageActiveContext value={active}>
      <PageKeepContext value={() => props.entry().keepAlive}>
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
            } catch {
              // Mock hosts without a scope stack still mount the page.
            }
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
          {transition({
            phase: props.entry().phase,
            onDone: () => props.router.complete(props.entry().key),
            children: (
              <Errored
                fallback={(err, reset) => {
                  const error = unwrapError(err);
                  console.error(loggedError(error));
                  const fallback = props.errorFallback ?? plainError;
                  return fallback(error, () => reset());
                }}
              >
                <Page params={props.entry().params} />
              </Errored>
            ),
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
            errorFallback={props.errorFallback}
          />
        )}
      </For>
    </stack>
  );
}
