import assert from "node:assert/strict";
import { matchPath, routePath } from "./path.ts";

assert.equal(routePath("https://example.test/nav/2?x=1"), "/nav/2");
assert.equal(routePath("schedule"), "/schedule");
assert.deepEqual(matchPath("/schedule", "/schedule"), {});
assert.equal(matchPath("/schedule", "/list"), null);
assert.deepEqual(matchPath("/nav/:depth", "/nav/3"), { depth: "3" });
assert.equal(matchPath("/nav/:depth", "/nav"), null);
assert.deepEqual(matchPath("/files/*", "/files/a/b"), { "*": "a/b" });
assert.deepEqual(matchPath("/files/*", "/files"), { "*": "" });
assert.equal(matchPath("/files/*/tail", "/files/a/tail"), null);

console.log("path.test.ts ok");
