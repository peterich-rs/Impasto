import { boot, scene } from "@impasto/solid-renderer";
import { createSignal } from "reactive";
import { command, report } from "impasto:test/driver";

const cases = [];
let scope = "";
let setContent;

report("start");
Promise.resolve().then(async () => {
  try {
    for (const [name, run] of cases) {
      report("case", name);
      await advanceFrames(1);
      await run();
      if (setContent) {
        setContent(() => null);
        await advanceFrames(1);
      }
      report("pass");
    }
    report("done");
  } catch (error) {
    report("fail", [String(error), error?.stack].filter(Boolean).join("\n"));
  }
});

export function test(name, run) {
  cases.push([scope + name, run]);
}

export function describe(name, register) {
  const previous = scope;
  scope += name + " / ";
  try {
    register();
  } finally {
    scope = previous;
  }
}

export function expect(actual) {
  function check(ok, expected) {
    if (!ok) throw new Error(`Expected ${JSON.stringify(actual)} ${expected}`);
  }
  return {
    toBe(expected) { check(Object.is(actual, expected), `to be ${JSON.stringify(expected)}`); },
    toEqual(expected) { check(equal(actual, expected), `to equal ${JSON.stringify(expected)}`); },
    toBeTruthy() { check(Boolean(actual), "to be truthy"); },
    toBeFalsy() { check(!actual, "to be falsy"); },
    toBeLessThan(expected) { check(actual < expected, `to be less than ${expected}`); },
    toBeGreaterThan(expected) { check(actual > expected, `to be greater than ${expected}`); },
    toBeVisible() {
      const { w, h } = scene.rect(actual);
      check(w > 0 && h > 0, "to be visible");
    },
  };
}

export const assert = {
  fail(message = "Assertion failed") { throw new Error(message); },
  ok(value, message) { if (!value) assert.fail(message ?? `Expected a truthy value, got ${value}`); },
  equal(actual, expected, message) {
    if (!Object.is(actual, expected)) assert.fail(message ?? `Expected ${String(expected)}, got ${String(actual)}`);
  },
  notEqual(actual, expected, message) {
    if (Object.is(actual, expected)) assert.fail(message ?? `Expected a value different from ${String(expected)}`);
  },
  deepEqual(actual, expected, message) {
    if (!equal(actual, expected)) assert.fail(message ?? `Expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  },
  match(actual, pattern, message) { assert.ok(pattern.test(actual), message ?? `Expected ${actual} to match ${pattern}`); },
  doesNotMatch(actual, pattern, message) { assert.ok(!pattern.test(actual), message ?? `Unexpected match for ${pattern}`); },
  throws(work, expected) {
    try { work(); } catch (error) {
      checkError(error, expected);
      return;
    }
    assert.fail("Expected an exception");
  },
  async rejects(work, expected) {
    try { await (typeof work === "function" ? work() : work); } catch (error) {
      checkError(error, expected);
      return;
    }
    assert.fail("Expected a rejection");
  },
  doesNotThrow(work) { work(); },
};

function checkError(error, expected) {
  if (!expected) return;
  if (expected instanceof RegExp) assert.match(String(error), expected);
  else if (expected === Error || expected.prototype instanceof Error) assert.ok(error instanceof expected);
  else assert.ok(expected(error), `Unexpected exception: ${error}`);
}

function equal(left, right, seen = new Map()) {
  if (Object.is(left, right)) return true;
  if (!left || !right || typeof left !== "object" || typeof right !== "object") return false;
  if (Object.getPrototypeOf(left) !== Object.getPrototypeOf(right)) return false;
  if (seen.has(left)) return seen.get(left) === right;
  seen.set(left, right);
  if (left instanceof Map) return left.size === right.size && Array.from(left).every(
    ([key, value]) => right.has(key) && equal(value, right.get(key), seen));
  if (left instanceof Set) return left.size === right.size && Array.from(left).every(value => right.has(value));
  if (left instanceof Date) return Object.is(left.getTime(), right.getTime());
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length && keys.every(key =>
    Object.hasOwn(right, key) && equal(left[key], right[key], seen));
}

export function advanceFrames(count = 1, step = 16) {
  return command("frames", count, step);
}

export const screen = {
  async mount(render) {
    if (!setContent) {
      const [get, set] = createSignal(render);
      setContent = set;
      boot(get);
    } else {
      setContent(render);
    }
    return advanceFrames(1);
  },
  byRole(role, { name } = {}) { return command("query", role, name ?? null); },
  byLabel(label) { return command("query", "", label); },
  byText(text) { return command("query", "text", text); },
  nodes(role = "", label = null) { return command("nodes", role, label); },
  rect: scene.rect,
  async viewport(width, height, scale = 1) {
    await command("viewport", width, height, scale);
    await advanceFrames();
  },
  snapshot(name) { return command("snapshot", name); },
  advanceFrames,
};

export const fire = {
  click(node) { return command("click", node); },
  focus(node) { scene.focus(node); return advanceFrames(); },
  wheel(node, dx, dy) { return command("wheel", node, dx, dy); },
  drag(node, dx, dy) { return command("drag", node, dx, dy); },
  back(phase = "commit", value = 0) { return command("back", phase, value); },
  replaceText(node, text) { return this.imeCompose(node, text, -1, -1); },
  imeCompose(node, text, start, end) {
    scene.focus(node);
    return command("replace", text, start, end);
  },
};

export async function trace(work) {
  await command("traceReset");
  await work();
  return command("traceRead");
}
