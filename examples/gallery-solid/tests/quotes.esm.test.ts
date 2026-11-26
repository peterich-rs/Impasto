import { assert } from "impasto:test";
import { readText } from "impasto:test/fixtures";

const source = readText("../dist/quotes.js");
assert.match(source, /from ["']scene["']/);
assert.match(source, /from ["']reactive["']/);
assert.equal(source.includes("@solidjs/universal"), false);
assert.equal(source.includes("REACTIVITY_HALTED"), false);
assert.equal(source.includes("export declare function"), false);
assert.equal(source.includes("TurboModuleRegistry"), false);
// Engine determinism guards survive minification-ish bundling.
assert.equal(source.includes("mulberry"), true);

console.log("quotes esm ok");
