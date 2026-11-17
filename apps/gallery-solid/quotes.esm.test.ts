import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("./dist/quotes.js", import.meta.url), "utf8");
assert.match(source, /from ["']scene["']/);
assert.match(source, /from ["']reactive["']/);
assert.equal(source.includes("@solidjs/universal"), false);
assert.equal(source.includes("REACTIVITY_HALTED"), false);
assert.equal(source.includes("export declare function"), false);
assert.equal(source.includes("TurboModuleRegistry"), false);
// Engine determinism guards survive minification-ish bundling.
assert.equal(source.includes("mulberry"), true);

console.log("quotes esm ok");
