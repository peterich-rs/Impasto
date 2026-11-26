import { assert, screen, test } from "impasto:test";
import {
  animate, flingTarget, layerTreeStats, scrollIntoView, setRecognizers,
} from "../../solid-renderer/src/guard.ts";
import { scene } from "@impasto/solid-renderer";

test("guards dispatch to the engine for a live node", async () => {
  const node = scene.createElement("view", { style: { height: 32 } });
  await screen.mount(() => node);
  scrollIntoView(node, 0);
  animate(node, "opacity", 0, 255, 170, 26);
  setRecognizers(node, { doubleTapMs: 280, drag: "none" });
  assert.equal(flingTarget(node), null);
  assert.ok(layerTreeStats().epoch > 0);
});
