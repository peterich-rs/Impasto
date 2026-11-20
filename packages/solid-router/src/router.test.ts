import assert from "node:assert/strict";
import { createRoot, flush } from "solid-js";
import { flushMotion, onBack, spring } from "@impasto/solid-renderer";
import { state as linking } from "../../linking/src/native.stub.ts";
import { createRouter, type RouteDef } from "./stack.ts";

const page = () => 0 as never;
const defs: RouteDef[] = [
  { path: "/schedule", component: page },
  { path: "/nav/:depth", component: page },
  { path: "/list", component: page },
  { path: "/files/*", component: page },
];

const backs: Array<() => boolean> = [];
let nativeBackEnabled = false;
let nativeGesture: ((phase: "start" | "progress" | "cancel" | "commit", value: number) => boolean | void) | undefined;
(globalThis as { core: Record<string, unknown> }).core = {
  onBack: (fn: () => boolean) => {
    backs.push(fn);
    return backs.length;
  },
  offBack: () => {},
  setBackEnabled: (enabled: boolean) => { nativeBackEnabled = enabled; },
  onBackGesture: (fn: typeof nativeGesture) => { nativeGesture = fn; },
  requestFrame: () => {},
};

{
  const saved = new Map([["routes", '[1,"/schedule","/nav/2"]']]);
  const restore = {
    key: "routes",
    get: (key: string) => saved.get(key) ?? null,
    set: (key: string, value: string | null) => {
      if (value === null) saved.delete(key);
      else saved.set(key, value);
    },
  };
  // Run in an owner so the predictive-return registration is released after this test.
  const dispose = createRoot((dispose) => {
    const router = createRouter(defs, { restoration: restore, linking: false });
    flush();
    assert.deepEqual(router.stack().map((entry) => entry.path), ["/schedule", "/nav/2"]);
    assert.equal(nativeBackEnabled, true);
    assert.ok(nativeGesture);
    nativeGesture("start", 1);
    nativeGesture("progress", 0.5);
    flush();
    assert.equal(router.backPreview()?.progress, 0.5);
    assert.equal(router.backPreview()?.fromRight, true);
    nativeGesture("cancel", 0);
    flush();
    assert.equal(router.backPreview(), null);
    assert.equal(saved.get("routes"), '[1,"/schedule","/nav/2"]');
    nativeGesture("start", 0);
    nativeGesture("commit", 1);
    flush();
    assert.equal(router.stack().at(-1)?.phase, "exit");
    assert.equal(saved.get("routes"), '[1,"/schedule"]');
    return dispose;
  });
  dispose();
  backs.length = 0;
  assert.equal(nativeBackEnabled, false);
}

{
  const calls: number[] = [];
  const high = onBack(300, () => { calls.push(300); return false; });
  const low = onBack(100, () => { calls.push(100); return true; });
  assert.ok(nativeGesture);
  nativeGesture("start", 0);
  assert.equal(nativeGesture("commit", 1), true);
  assert.deepEqual(calls, [300, 100]);
  low();
  nativeGesture("start", 0);
  assert.equal(nativeGesture("commit", 1), false);
  high();
  backs.length = 0;
}

async function settleLinking(): Promise<void> {
  await import("@impasto/linking");
  await Promise.resolve();
}

const router = createRouter(defs, {
  initial: "/schedule",
  onBack: () => false,
});
await settleLinking();
assert.equal(backs.length, 1);
assert.equal(router.stack()[0]?.path, "/schedule");
assert.equal(router.stack()[0]?.phase, "live");
assert.equal(router.push("/missing"), false);
assert.equal(router.push("/nav/1"), true);
flush();
assert.equal(router.depth(), 2);
assert.equal(router.stack()[0]?.phase, "cover");
assert.equal(router.stack()[1]?.phase, "enter");
assert.equal(router.stack()[1]?.params.depth, "1");
const covered = router.stack()[0];
assert.ok(covered);
router.complete(covered);
flush();
assert.equal(router.depth(), 2);
assert.equal(router.stack()[0]?.phase, "live");
assert.equal(router.stack()[0]?.key, covered.key);

const top = router.stack()[1];
assert.ok(top);
assert.equal(router.pop(), true);
flush();
assert.equal(router.depth(), 2);
assert.equal(router.stack()[0]?.phase, "live");
assert.equal(router.stack()[1]?.phase, "exit");
assert.equal(router.stack()[1]?.key, top.key);
const exiting = router.stack()[1];
assert.ok(exiting);

let removed = false;
spring({
  id: 1,
  prop: "translateX",
  from: 0,
  to: -30,
  onDone: () => {
    removed = true;
    router.complete(exiting);
  },
});
assert.equal(removed, false);
assert.equal(router.depth(), 2);
flushMotion();
flush();
assert.equal(removed, true);
assert.equal(router.depth(), 1);
assert.equal(router.stack()[0]?.path, "/schedule");

const handler = backs[0];
assert.ok(handler);
router.push("/list");
flush();
assert.equal(handler(), true);
flush();
assert.equal(router.stack()[1]?.phase, "exit");
const exitingList = router.stack()[1];
assert.ok(exitingList);
router.complete(exitingList);
flush();
assert.equal(handler(), false);

linking.urlListener?.({ url: "/nav/4" });
flush();
assert.equal(router.stack().at(-1)?.path, "/nav/4");
assert.equal(router.stack().at(-1)?.phase, "enter");

// complete() must not overwrite a push that landed in the same flush.
{
  const race = createRouter(defs, { initial: "/schedule" });
  race.push("/nav/1");
  flush();
  const covered = race.stack()[0];
  assert.ok(covered);
  race.complete(covered);
  assert.equal(race.push("/list"), true);
  flush();
  assert.equal(race.stack().at(-1)?.path, "/list");
  assert.equal(race.stack().length, 3);
}

{
  const race = createRouter(defs, { initial: "/schedule" });
  race.push("/nav/1");
  flush();
  const entering = race.stack().at(-1);
  assert.ok(entering);
  assert.equal(race.pop(), true);
  flush();
  const first = race.stack().at(-1);
  assert.ok(first);
  assert.equal(first.phase, "exit");
  race.complete(entering);
  flush();
  assert.equal(race.stack().at(-1), first);
  assert.equal(race.push("/nav/2"), true);
  flush();
  assert.equal(race.push("/nav/2"), false);
  race.complete(first);
  flush();
  assert.deepEqual(race.stack().map((entry) => entry.path), ["/schedule", "/nav/2"]);
  assert.equal(race.pop(), true);
  flush();
  const second = race.stack().at(-1);
  assert.ok(second);
  race.complete(second);
  flush();
  assert.deepEqual(race.stack().map((entry) => entry.path), ["/schedule"]);
}

{
  const race = createRouter(defs, { initial: "/schedule" });
  assert.equal(race.replace("/list"), true);
  assert.equal(race.pop(), false, "the retiring root is out of history");
  assert.equal(race.push("/nav/1"), true);
  assert.equal(race.push("/nav/2"), true);
  assert.equal(race.popTo("/list"), true);
  flush();
  assert.deepEqual(race.stack().map(({ path, phase }) => [path, phase]), [
    ["/list", "live"],
    ["/nav/1", "exit"],
    ["/nav/2", "exit"],
  ]);
  for (const entry of race.stack().slice(1)) race.complete(entry);
  flush();
  assert.equal(race.depth(), 1);
}

linking.initialUrl = null;
const deep = createRouter(defs, { initial: "/schedule" });
await settleLinking();
flush();
assert.equal(deep.stack()[0]?.path, "/schedule");

linking.initialUrl = "/list";
const linked = createRouter(defs, { initial: "/schedule" });
await settleLinking();
flush();
assert.equal(linked.stack()[0]?.path, "/list");
assert.equal(linked.stack()[0]?.phase, "live");

const later = createRouter(defs, { initial: "/schedule" });
await settleLinking();
later.push("/nav/9");
flush();
assert.equal(handler(), true);
flush();
assert.equal(later.stack().at(-1)?.phase, "exit");
assert.equal(router.stack().at(-1)?.path, "/nav/4");

console.log("router.test.ts ok");
