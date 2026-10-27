import assert from "node:assert/strict";
import { directionFor } from "./direction.ts";

for (const locale of ["ar", "he", "fa", "ur", "ar-EG"]) {
  assert.equal(directionFor(locale), "rtl");
}
assert.equal(directionFor("en"), "ltr");
assert.equal(directionFor("zh"), "ltr");

console.log("direction.test.ts ok");
