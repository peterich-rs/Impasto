import { advanceFrames, assert, fire, test } from "impasto:test";
import { createRoot } from "solid-js";
import { onBack } from "@impasto/solid-renderer";
import { createRouter, type RouteDef } from "./stack.ts";

const page = () => 0 as never;
const defs: RouteDef[] = [
  { path: "/schedule", component: page },
  { path: "/nav/:depth", component: page },
  { path: "/list", component: page },
  { path: "/files/*", component: page },
];

test("route restoration, predictive back and stack races", async () => {
  const saved = new Map([["routes", '[1,"/schedule","/nav/2"]']]);
  const restore = {
    key: "routes",
    get: (key: string) => saved.get(key) ?? null,
    set: (key: string, value: string | null) => {
      if (value === null) saved.delete(key);
      else saved.set(key, value);
    },
  };
  const { router, dispose } = createRoot(dispose => ({
    router: createRouter(defs, { restoration: restore, linking: false }),
    dispose,
  }));
  assert.deepEqual(router.stack().map(entry => entry.path), ["/schedule", "/nav/2"]);
  await fire.back("start", 1);
  await fire.back("progress", 0.5);
  assert.equal(router.backPreview()?.progress, 0.5);
  assert.equal(router.backPreview()?.fromRight, true);
  await fire.back("cancel");
  assert.equal(router.backPreview(), null);
  assert.equal(saved.get("routes"), '[1,"/schedule","/nav/2"]');
  await fire.back("start");
  assert.equal(await fire.back(), true);
  assert.equal(router.stack().at(-1)?.phase, "exit");
  assert.equal(saved.get("routes"), '[1,"/schedule"]');
  dispose();

  const calls: number[] = [];
  const high = onBack(300, () => { calls.push(300); return false; });
  const low = onBack(100, () => { calls.push(100); return true; });
  await fire.back("start");
  assert.equal(await fire.back(), true);
  assert.deepEqual(calls, [300, 100]);
  low();
  high();

  // complete() must not overwrite a push that landed in the same flush.
  {
    const race = createRouter(defs, { initial: "/schedule", linking: false });
    race.push("/nav/1");
    await advanceFrames();
    const covered = race.stack()[0];
    assert.ok(covered);
    race.complete(covered);
    assert.equal(race.push("/list"), true);
    await advanceFrames();
    assert.equal(race.stack().at(-1)?.path, "/list");
    assert.equal(race.stack().length, 3);
  }

  {
    const race = createRouter(defs, { initial: "/schedule", linking: false });
    race.push("/nav/1");
    await advanceFrames();
    const entering = race.stack().at(-1);
    assert.ok(entering);
    assert.equal(race.pop(), true);
    await advanceFrames();
    const first = race.stack().at(-1);
    assert.ok(first);
    assert.equal(first.phase, "exit");
    race.complete(entering);
    await advanceFrames();
    assert.equal(race.stack().at(-1), first);
    assert.equal(race.push("/nav/2"), true);
    await advanceFrames();
    assert.equal(race.push("/nav/2"), false);
    race.complete(first);
    await advanceFrames();
    assert.deepEqual(race.stack().map((entry) => entry.path), ["/schedule", "/nav/2"]);
    assert.equal(race.pop(), true);
    await advanceFrames();
    const second = race.stack().at(-1);
    assert.ok(second);
    race.complete(second);
    await advanceFrames();
    assert.deepEqual(race.stack().map((entry) => entry.path), ["/schedule"]);
  }

  {
    const race = createRouter(defs, { initial: "/schedule", linking: false });
    assert.equal(race.replace("/list"), true);
    assert.equal(race.pop(), false, "the retiring root is out of history");
    assert.equal(race.push("/nav/1"), true);
    assert.equal(race.push("/nav/2"), true);
    assert.equal(race.popTo("/list"), true);
    await advanceFrames();
    assert.deepEqual(race.stack().map(({ path, phase }) => [path, phase]), [
      ["/list", "live"],
      ["/nav/1", "exit"],
      ["/nav/2", "exit"],
    ]);
    for (const entry of race.stack().slice(1)) race.complete(entry);
    await advanceFrames();
    assert.equal(race.depth(), 1);
  }
});
