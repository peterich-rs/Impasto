import assert from "node:assert/strict";
import { resolvePadding } from "./layout.ts";

assert.deepEqual(resolvePadding("ltr", 8, 4), { left: 8, right: 4 });
assert.deepEqual(resolvePadding("rtl", 8, 4), { left: 4, right: 8 });
assert.deepEqual(resolvePadding("rtl", 16, 0), { left: 0, right: 16 });

console.log("layout.test.ts ok");
