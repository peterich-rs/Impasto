import { createRenderEffect, createSignal, createTrackedEffect } from "solid-js";
import { host, type NodeHandle } from "@uifw/solid-renderer";
import { pullDelta } from "./list-window.ts";

/**
 * A vertical-and-horizontal scroller. Drag and wheel update the offset here.
 * After release, `core` coasts and reports the offset through `onScroll`.
 *
 * Offset is clamped to the gap between the content box and the viewport.
 */
export function ScrollArea(props: {
  children?: unknown;
  onOffset?: (offset: { x: number; y: number }, viewport: number) => void;
  viewportRef?: (viewport: number) => void;
  onPull?: (delta: number, overflowing: boolean) => void;
  onDragEnd?: () => void;
  initialOffset?: { x: number; y: number };
  /** Minimum content height. Keeps the viewport scrollable when row layout lags the window. */
  contentMin?: number;
}): NodeHandle {
  const [off, setOff] = createSignal(props.initialOffset ?? { x: 0, y: 0 });
  let viewport = 0;
  let content = 0;
  let contentMin = 0;

  function applyContentMin(): void {
    if (content !== 0 && Number.isFinite(contentMin) && contentMin > 0) {
      host.setMinSize(content, 0, contentMin);
    }
  }

  function clamp(x: number, y: number): { x: number; y: number } {
    if (viewport === 0 || content === 0) {
      return { x: 0, y: 0 };
    }
    const view = host.rect(viewport);
    const body = host.rect(content);
    const maxX = Math.max(0, body.w - view.w);
    const maxY = Math.max(0, body.h - view.h, contentMin - view.h);
    return {
      x: Math.min(maxX, Math.max(0, x)),
      y: Math.min(maxY, Math.max(0, y)),
    };
  }

  function overflowing(): boolean {
    if (viewport === 0 || content === 0) {
      return false;
    }
    const view = host.rect(viewport);
    const body = host.rect(content);
    return body.h > view.h;
  }

  createRenderEffect(off, (next) => {
    if (viewport !== 0) {
      host.setScrollOffset(viewport, next.x, next.y);
      props.onOffset?.(next, viewport);
    }
  });

  createTrackedEffect(() => {
    contentMin = props.contentMin ?? 0;
    applyContentMin();
  });

  return (
    <scroll
      fill
      ref={(node) => {
        viewport = node.id;
        props.viewportRef?.(node.id);
      }}
      onScroll={(x, y) => {
        setOff(clamp(x, y));
      }}
      onDragBy={(dx, dy) => {
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        const rawY = live.y - dy;
        const next = clamp(live.x - dx, rawY);
        props.onPull?.(pullDelta(live.y, rawY, next.y), overflowing());
        setOff(next);
      }}
      onDragEnd={() => {
        props.onDragEnd?.();
      }}
      onWheel={(dx, dy) => {
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        setOff(clamp(live.x + dx, live.y + dy));
      }}
    >
      <column
        ref={(node) => {
          content = node.id;
          applyContentMin();
        }}
      >
        {props.children}
      </column>
    </scroll>
  );
}
