import assert from "node:assert/strict";

const logs: string[] = [];
(globalThis as { core: Record<string, unknown> }).core = {
  log: (level: string, message: string) => {
    logs.push(`${level}:${message}`);
  },
  setListenerErrorPolicy: () => {},
};

const { reportError, installErrorHooks } = await import("./errors.ts");
reportError(new Error("boom"));
assert.deepEqual(logs, ["error:boom"]);
const stop = installErrorHooks();
assert.equal(typeof stop, "function");
stop();

console.log("errors.test.ts ok");
