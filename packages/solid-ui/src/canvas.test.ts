import assert from "node:assert/strict";
import { createRoot, flush } from "solid-js";
import { Canvas } from "./Canvas.tsx";

const calls: number[] = [];
const returns: unknown[] = [];
const errorHandlers: Array<((error: unknown) => unknown) | undefined> = [];
let released = 0;
let controller: { release(): void } | null = null;
(globalThis as { core: Record<string, unknown> }).core = {
  view: () => 7,
  createStyleSource: () => 1,
  setStyleSource: () => {},
  removeStyleSource: () => {},
  bindCanvas: (
    node: number,
    draw: (context: unknown, size: { width: number; height: number }) => unknown,
    onError?: (error: unknown) => unknown,
  ) => {
    calls.push(node);
    returns.push(draw(null, { width: 1, height: 1 }));
    errorHandlers.push(onError);
    return {
      invalidate() {},
      toLocal() {
        return null;
      },
      release() {
        released += 1;
      },
    };
  },
};

createRoot((dispose) => {
  const node = Canvas({
    draw: () => "sync",
    onDrawError: () => {},
    controller: (next) => {
      controller = next;
    },
  });
  let handled = 0;
  let handler: (() => void) | undefined;
  Canvas({
    draw: async () => "later",
    get onDrawError() { return handler; },
  });
  flush();
  assert.equal(node, 7);
  assert.deepEqual(calls, [7, 7]);
  assert.equal(returns[0], "sync");
  assert.equal(typeof (returns[1] as Promise<string>)?.then, "function");
  assert.equal(errorHandlers[0]?.({}), undefined);
  assert.equal(errorHandlers[1]?.({}), false);
  handler = () => { handled += 1; };
  assert.equal(errorHandlers[1]?.({}), undefined);
  assert.equal(handled, 1);
  assert.equal(typeof controller?.release, "function");
  dispose();
});

assert.equal(released, 2);
assert.equal(controller, null);
console.log("canvas.test.ts ok");
