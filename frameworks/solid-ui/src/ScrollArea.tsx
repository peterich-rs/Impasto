import { createRenderEffect, createSignal } from "solid-js";
import { host, onFrame, requestFrame, type NodeHandle } from "@uifw/solid-renderer";

const DECAY = 0.95;
const STOP = 0.05;

/**
 * A vertical-and-horizontal scroller. Drag follows the finger. After release,
 * velocity decays until it falls under {@link STOP} scene points per millisecond.
 *
 * Offset is clamped to the gap between the content box and the viewport. Both
 * sizes come from `core.rect` after layout. The tree does not run the decay.
 */
export function ScrollArea(props: { children?: unknown }): NodeHandle {
  const [off, setOff] = createSignal({ x: 0, y: 0 });
  let viewport = 0;
  let content = 0;
  let velocityX = 0;
  let velocityY = 0;
  let last = 0;
  let coasting = false;

  function clamp(x: number, y: number): { x: number; y: number } {
    if (viewport === 0 || content === 0) {
      return { x: 0, y: 0 };
    }
    const view = host.rect(viewport);
    const body = host.rect(content);
    const maxX = Math.max(0, body.w - view.w);
    const maxY = Math.max(0, body.h - view.h);
    return {
      x: Math.min(maxX, Math.max(0, x)),
      y: Math.min(maxY, Math.max(0, y)),
    };
  }

  onFrame((timeMs) => {
    if (!coasting || viewport === 0) {
      return;
    }
    const dt = last === 0 ? 16 : Math.min(34, Math.max(0, timeMs - last));
    last = timeMs;
    velocityX *= DECAY;
    velocityY *= DECAY;
    setOff(clamp(off().x - velocityX * dt, off().y - velocityY * dt));
    if (Math.abs(velocityX) < STOP && Math.abs(velocityY) < STOP) {
      coasting = false;
      return;
    }
    requestFrame();
  });

  createRenderEffect(off, (next) => {
    if (viewport !== 0) {
      host.setScrollOffset(viewport, next.x, next.y);
    }
  });

  return (
    <scroll
      ref={(node) => {
        viewport = node.id;
      }}
      onDragBy={(dx, dy) => {
        coasting = false;
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        setOff(clamp(live.x - dx, live.y - dy));
      }}
      onWheel={(dx, dy) => {
        coasting = false;
        const live = viewport === 0 ? off() : host.scrollOffset(viewport);
        setOff(clamp(live.x + dx, live.y + dy));
      }}
      onDragEnd={(vx, vy) => {
        velocityX = vx;
        velocityY = vy;
        last = 0;
        if (Math.abs(vx) < STOP && Math.abs(vy) < STOP) {
          return;
        }
        coasting = true;
        requestFrame();
      }}
    >
      <column
        ref={(node) => {
          content = node.id;
        }}
      >
        {props.children}
      </column>
    </scroll>
  );
}
