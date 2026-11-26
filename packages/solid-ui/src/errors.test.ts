import { assert, test } from "impasto:test";
import { formatError, installErrorHooks, messageOf, stackOf } from "./errors.ts";

test("formats QuickJS errors and removes the installed listener", () => {
  const error = new Error("boom");
  assert.match(formatError(error), /boom/);
  assert.match(formatError(error), /errors\.test\.ts/);
  assert.equal(messageOf(error), "boom");
  assert.ok(stackOf(error).length);
  const stop = installErrorHooks();
  stop();
});
