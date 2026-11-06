import { createSignal, onCleanup, type Accessor } from "solid-js";
import { animate, host, onFrame, requestFrame, type NodeHandle } from "@impasto/solid-renderer";
import {
  PULL_DAMPING,
  PULL_STIFFNESS,
  gapOpen,
  pullProgress,
  pullRelease,
  quantizeProgress,
  type PullEdge,
} from "./pull.ts";
import type { ScrollController } from "./ScrollController.ts";

export type PullState = {
  progress: Accessor<number>;
  busy: Accessor<boolean>;
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
export function RefreshControl(props: {
  controller: ScrollController;
  threshold?: number;
  edge?: PullEdge;
  onRefresh: () => void | Promise<void>;
  gesture: (handlers: { onDragBy: () => void; onDragEnd: () => void }) => void;
  children: (state: PullState) => NodeHandle;
}): NodeHandle {
  const edge = () => props.edge ?? "start";
  const threshold = () => props.threshold ?? 64;
  const [progress, setProgress] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  let closing = false;

  function sample(): number {
    const node = props.controller.node();
    if (node === 0) {
      return 0;
    }
    return host.overscroll(node).y;
  }

  function sync(overscroll = sample()): void {
    if (busy() && !closing) {
      setProgress(1);
      return;
    }
    setProgress(quantizeProgress(pullProgress(edge(), overscroll, threshold())));
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
    animate(node, "overscrollY", overscroll, target, PULL_STIFFNESS, PULL_DAMPING);
    requestFrame();
  }

  function finish(): void {
    setBusy(false);
    const node = props.controller.node();
    const overscroll = sample();
    if (node !== 0 && gapOpen(edge(), overscroll)) {
      closing = true;
      animate(node, "overscrollY", overscroll, 0, PULL_STIFFNESS, PULL_DAMPING);
      requestFrame();
      return;
    }
    closing = false;
    setProgress(0);
  }

  function onDragBy(): void {
    if (closing) {
      closing = false;
    }
    sync();
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

  props.gesture({ onDragBy, onDragEnd });
  onCleanup(
    onFrame(() => {
      const overscroll = sample();
      if (closing && !gapOpen(edge(), overscroll)) {
        closing = false;
        setProgress(0);
        return;
      }
      sync(overscroll);
      if (busy() || closing) {
        requestFrame();
      }
    }),
  );

  return (
    <column
      absolute
      insetTop={-threshold()}
      insetLeft={0}
      w="100%"
      h={threshold()}
      justify="center"
      cross="center"
    >
      {props.children({ progress, busy })}
    </column>
  ) as unknown as NodeHandle;
}
