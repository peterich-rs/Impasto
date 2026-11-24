import { createSignal, onCleanup, type Accessor } from "solid-js";
import {
  animate,
  bindNodeRef,
  bindStyle,
  scene,
  onFrame,
  requestFrame,
  type NodeHandle,
  type StyleInput,
  Text,
} from "@impasto/solid-renderer";
import type { RefreshControlProps } from "./props.ts";
import {
  PULL_DAMPING,
  PULL_STIFFNESS,
  gapOpen,
  pullProgress,
  pullRelease,
  quantizeProgress,
  type PullEdge,
} from "./pull.ts";
import { useScrollThreshold } from "./scroll-rules.ts";
import { bindScrollStyle, interpolate } from "./scroll-style.ts";
import { useTheme } from "./use-theme.ts";
export type PullState = {
  progress: Accessor<number>;
  busy: Accessor<boolean>;
  ready: Accessor<boolean>;
};

/**
 * Holds a scroll's overscroll at the threshold while `onRefresh` runs, then
 * springs it shut.
 *
 * The indicator is a child of the scroll content, absolutely positioned one
 * threshold above the content origin. The viewport clips it. Negative
 * overscroll moves the content and the indicator together, so the word only
 * enters the viewport while the list is pulled down.
 */
export function RefreshControl(props: RefreshControlProps): NodeHandle {
  const edge = () => props.edge ?? "start";
  const threshold = () => Math.max(1, props.threshold ?? 64);
  const theme = useTheme();
  const [progress, setProgress] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const ready = useScrollThreshold(props.controller, {
    axis: "overscrollY",
    get value() { return edge() === "start" ? -threshold() : threshold(); },
  });
  let closing = false;
  let observesProgress = false;
  const readProgress = () => {
    observesProgress = true;
    return progress();
  };
  let disposed = false;
  let stopFrames: (() => void) | undefined;
  function endSampling(): void {
    stopFrames?.();
    stopFrames = undefined;
  }
  function sampleCustomIndicator(): void {
    if (!observesProgress || stopFrames) return;
    stopFrames = onFrame(() => {
      const overscroll = sample();
      if (!gapOpen(edge(), overscroll)) {
        closing = false;
        setProgress(0);
        endSampling();
        return;
      }
      sync(overscroll);
      requestFrame();
    });
    requestFrame();
  }
  function sample(): number {
    const node = props.controller.node();
    if (node === 0) {
      return 0;
    }
    return scene.overscroll(node).y;
  }
  function sync(overscroll = sample()): void {
    if (busy() && !closing) {
      setProgress(1);
      return;
    }
    setProgress(
      quantizeProgress(pullProgress(edge(), overscroll, threshold())),
    );
  }
  function park(overscroll: number): void {
    const node = props.controller.node();
    if (node === 0) {
      return;
    }
    const target = edge() === "start" ? -threshold() : threshold();
    props.controller.stop();
    if (Math.abs(overscroll - target) < 0.5) {
      return;
    }
    animate(
      node,
      "overscrollY",
      overscroll,
      target,
      PULL_STIFFNESS,
      PULL_DAMPING,
    );
    requestFrame();
  }
  function finish(): void {
    if (disposed) return;
    setBusy(false);
    const node = props.controller.node();
    const overscroll = sample();
    if (node !== 0 && gapOpen(edge(), overscroll)) {
      closing = props.children !== undefined;
      sampleCustomIndicator();
      animate(node, "overscrollY", overscroll, 0, PULL_STIFFNESS, PULL_DAMPING);
      requestFrame();
      return;
    }
    closing = false;
    setProgress(0);
  }
  function onDragBy(): void {
    endSampling();
    if (closing) {
      closing = false;
    }
    if (props.children !== undefined) sync();
  }
  function onDragEnd(): void {
    const overscroll = sample();
    const decision = pullRelease({
      edge: edge(),
      overscroll,
      threshold: threshold(),
      busy: busy(),
    });
    if (decision === "ignore") {
      if (gapOpen(edge(), overscroll)) sampleCustomIndicator();
      return;
    }
    setProgress(1);
    park(overscroll);
    if (decision === "hold") {
      return;
    }
    setBusy(true);
    requestFrame();
    Promise.resolve()
      .then(() => props.onRefresh())
      .then(finish, finish);
  }
  props.gesture({
    onDragBy,
    onDragEnd,
  });
  onCleanup(() => {
    disposed = true;
    endSampling();
    props.gesture({ onDragBy: () => {}, onDragEnd: () => {} });
  });
  return (
    <view
      ref={(node) => {
        if (node === null) return;
        bindStyle(node, {
          get defaults(): StyleInput {
            return {
              position: "absolute",
              ...(edge() === "start" ? { top: -threshold() } : { bottom: -threshold() }),
              left: 0,
              width: "100%",
              height: threshold(),
              justifyContent: "center",
              alignItems: "center",
            };
          },
          get class() {
            return props.class;
          },
          get classList() {
            return props.classList;
          },
          get style() {
            return props.style;
          },
        });
        bindNodeRef(node, () => props.ref);
      }}
    >
      {props.children?.({ progress: readProgress, busy, ready }) ?? (
        <view
          ref={(node) => {
            if (node === null) return;
            const start = edge() === "start";
            const span = threshold();
            const input = start ? [-span, 0] : [0, span];
            onCleanup(bindScrollStyle(node, props.controller, {
              source: "overscrollY",
              translateY: interpolate(input, start ? [0, 8] : [-8, 0]),
              opacity: interpolate(input, start ? [1, 0] : [0, 1]),
              rotate: interpolate(input, start ? [0, -0.12] : [0.12, 0]),
            }));
          }}
        >
          <Text style={{ color: theme().color.textMuted, fontSize: theme().fontSize.body }}>
            {theme().locale.startsWith("zh")
              ? (busy() ? "正在刷新" : ready() ? "松开刷新" : "下拉刷新")
              : (busy() ? "Refreshing…" : ready() ? "Release to refresh" : "Pull to refresh")}
          </Text>
        </view>
      )}
    </view>
  ) as unknown as NodeHandle;
}
