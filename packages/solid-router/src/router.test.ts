import assert from "node:assert/strict";
import { flush } from "solid-js";
import { flushMotion, spring } from "@impasto/solid-renderer";
import { state as linking } from "@impasto/linking/host";
import { createRouter, type RouteDef } from "./stack.ts";

const page = () => 0 as never;
const defs: RouteDef[] = [
  { path: "/schedule", component: page },
  { path: "/nav/:depth", component: page },
  { path: "/list", component: page },
  { path: "/files/*", component: page },
];

const backs: Array<() => boolean> = [];
(globalThis as { core: Record<string, unknown> }).core = {
  onBack: (fn: () => boolean) => {
    backs.push(fn);
    return backs.length;
  },
  offBack: () => {},
  requestFrame: () => {},
};

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
router.complete(covered.key);
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

let removed = false;
spring({
  id: 1,
  prop: "translateX",
  from: 0,
  to: -30,
  onDone: () => {
    removed = true;
    router.complete(top.key);
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
router.complete(router.stack()[1]?.key ?? "");
flush();
assert.equal(handler(), false);

linking.urlListener?.({ url: "/nav/4" });
flush();
assert.equal(router.stack().at(-1)?.path, "/nav/4");
assert.equal(router.stack().at(-1)?.phase, "enter");

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
