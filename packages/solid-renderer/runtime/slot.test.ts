import { assert, screen, test, trace } from "impasto:test";
import { children, createRenderEffect, createSignal, createTrackedEffect } from "solid-js";
import { createSlot, createComponent, insert, scene, Text } from "./index.ts";

test("slots keep native text and owners across updates, moves and detachment", async () => {
  const [flag, setFlag] = createSignal(false);
  const seen = { active: false, reads: 0 };
  const parent = scene.createElement("view");
  const slot = createSlot(1, 0, (item) => {
    const text = scene.createTextNode("row");
    createTrackedEffect(() => {
      seen.reads++;
      scene.setText(text, `Item ${item()}`);
    });
    createTrackedEffect(() => { seen.active = flag(); });
    return text;
  });
  slot.attach(parent);
  await screen.mount(() => parent);
  const first = await screen.byText("Item 1");
  const stats = await trace(async () => {
    setFlag(true);
    slot.set(2, 1);
    slot.place(parent);
    await screen.advanceFrames();
  });
  assert.equal(seen.active, true);
  assert.ok(seen.reads >= 2);
  assert.equal(stats.createdNodes, 0);
  assert.equal(await screen.byText("Item 2"), first);
  slot.detach();
  assert.equal(scene.parent(slot.node), 0);
  slot.set(3, 2);
  slot.place(parent);
  await screen.advanceFrames();
  assert.equal(await screen.byText("Item 3"), first);
  slot.discard();
  await screen.advanceFrames();
  assert.equal((await screen.nodes("text", "Item 3")).length, 0);
});

test("signals update a slot created in a rerunning owner", async () => {
  const parent = scene.createElement("view");
  const [generation, setGeneration] = createSignal(1);
  let owned: ReturnType<typeof createSlot<number>> | undefined;
  await screen.mount(() => {
    createTrackedEffect(() => {
      const n = generation();
      if (owned) {
        owned.set(n, n);
        return;
      }
      owned = createSlot(n, 0, item => {
        const row = scene.createElement("view");
        const text = scene.createTextNode("");
        const resolved = children(() => `Raw ${item()}`);
        createRenderEffect(resolved, value => scene.setText(text, String(value)));
        scene.insertNode(row, text);
        insert(row, createComponent(Text, { get children() { return `Item ${item()}`; } }));
        return row;
      });
      owned.attach(parent);
    });
    return parent;
  });
  const first = await screen.byText("Item 1");
  setGeneration(9);
  await screen.advanceFrames();
  assert.equal(await screen.byText("Item 9"), first);
  assert.ok(await screen.byText("Raw 9"));
  owned?.discard();
});
