import { advanceFrames, assert, screen, test } from "impasto:test";
import { scene } from "./scene.ts";
import { spring } from "./motion.ts";

test("spring completes on a native frame and cancellation removes the handler", async () => {
  const node = scene.createElement("view");
  await screen.mount(() => node);
  let settled = false;
  spring({ id: node, prop: "opacity", from: 0, to: 255, onDone: () => { settled = true; } });
  assert.equal(settled, false);
  await advanceFrames(120);
  assert.equal(settled, true);

  const cancel = spring({
    id: node, prop: "opacity", from: 255, to: 0,
    onDone: () => assert.fail("cancelled motion completed"),
  });
  cancel();
  await advanceFrames(120);
});
