import assert from "node:assert/strict";
import {
  animate,
  flingTarget,
  layerTreeStats,
  onShortcut,
  scrollIntoView,
  setListenerErrorPolicy,
  setRecognizers,
} from "../../solid-renderer/src/guard.ts";

(globalThis as { core: Record<string, unknown> }).core = {};
scrollIntoView(1, 0);
animate(1, "opacity", 0, 255, 170, 26);
onShortcut("mod+k", () => {});
setListenerErrorPolicy("freeze");
setRecognizers(1, { doubleTapMs: 280, drag: "none" });
assert.equal(flingTarget(1), null);
assert.deepEqual(layerTreeStats(), { epoch: 0, layers: 0, pictures: 0 });

const seen: unknown[] = [];
(globalThis as { core: Record<string, unknown> }).core = {
  setRecognizers: (_id: number, spec: unknown) => {
    seen.push(spec);
  },
  flingTarget: () => ({ x: 1, y: 40 }),
};
setRecognizers(4, { doubleTapMs: 280 });
assert.deepEqual(seen[0], { doubleTapMs: 280 });
assert.deepEqual(flingTarget(4), { x: 1, y: 40 });

console.log("guard.test.ts ok");
