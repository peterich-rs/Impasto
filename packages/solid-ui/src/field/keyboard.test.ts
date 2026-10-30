import assert from "node:assert/strict";
import { enterInsertsNewline, imeConfig } from "./keyboard.ts";

assert.deepEqual(imeConfig(undefined, undefined, false), { keyboard: 0, action: 1 });
assert.deepEqual(imeConfig(undefined, undefined, true), { keyboard: 1, action: 7 });
assert.deepEqual(imeConfig("phone", "search", false), { keyboard: 3, action: 3 });
assert.deepEqual(imeConfig("number", "newline", true), { keyboard: 2, action: 7 });
assert.deepEqual(imeConfig("email", "go", false), { keyboard: 4, action: 4 });
assert.deepEqual(imeConfig("url", "send", false), { keyboard: 5, action: 5 });
assert.deepEqual(imeConfig("name", "next", false), { keyboard: 6, action: 2 });
assert.deepEqual(imeConfig("password", "previous", false), { keyboard: 7, action: 6 });
assert.equal(enterInsertsNewline(true, "newline"), true);
assert.equal(enterInsertsNewline(true, undefined), true);
assert.equal(enterInsertsNewline(true, "done"), false);
assert.equal(enterInsertsNewline(true, "search"), false);
assert.equal(enterInsertsNewline(false, "newline"), false);

console.log("keyboard.test.ts ok");
