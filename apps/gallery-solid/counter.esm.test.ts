import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { kicker } from "./util.js";

assert.equal(kicker(), "Impasto");
const source = readFileSync(new URL("./dist/counter.js", import.meta.url), "utf8");
assert.match(source, /import\(["']@impasto\/linking\/host["']\)/);
assert.equal(source.includes("export declare function"), false);
assert.equal(source.includes("TurboModuleRegistry"), false);

console.log("gallery esm ok");
