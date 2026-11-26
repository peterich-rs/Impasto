import { assert, screen, test } from "impasto:test";
import { Canvas } from "./Canvas.tsx";

test("Canvas delivers a native drawing context and releases the controller", async () => {
  let size = 0;
  let controller: { release(): void } | null = null;
  await screen.mount(() => Canvas({
    style: { width: 50, height: 40 },
    draw(ctx, bounds) {
      size = bounds.width * bounds.height;
      ctx.fillRect(0, 0, 10, 10);
    },
    controller(next) { controller = next; },
  }));
  assert.equal(size, 50 * 40);
  assert.ok(controller);
  await screen.mount(() => null);
  assert.equal(controller, null);
});

test("Canvas reports an asynchronous draw callback", async () => {
  let error: { message: string } | undefined;
  await screen.mount(() => Canvas({
    style: { width: 50, height: 40 },
    async draw() {},
    onDrawError(value) { error = value; },
  }));
  await screen.advanceFrames(2);
  assert.match(error?.message ?? "", /sync/i);
});
