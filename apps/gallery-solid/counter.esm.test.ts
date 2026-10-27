import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { kicker } from "./dist/util.js";

assert.equal(kicker(), "Future");
const source = readFileSync(new URL("./dist/counter.esm.js", import.meta.url), "utf8");
assert.match(source, /from\s*["']\.\/util\.js["']/);
assert.equal(source.includes("function kicker"), false);

console.log("gallery esm ok");
