import assert from "node:assert/strict";
import { directionFor, formatNumber, messagesFor, plural } from "./i18n.ts";

assert.deepEqual(messagesFor("zh-CN"), { switchOn: "开", switchOff: "关" });
assert.equal(messagesFor("en").switchOn, "On");
assert.equal(messagesFor("fr").switchOff, "Off");

assert.equal(plural("en", 1, { one: "item", other: "items" }), "item");
assert.equal(plural("en", 2, { one: "item", other: "items" }), "items");
assert.equal(plural("zh", 1, { one: "item", other: "items" }), "items");

assert.equal(formatNumber("en", 1.25, 1), "1.3");
assert.equal(formatNumber("en", -1.25, 1), "-1.3");
assert.equal(formatNumber("en", 1234.5, 1), "1,234.5");
assert.equal(formatNumber("en", Number.NaN, 1), "");
assert.equal(formatNumber("zh", 1234.5, 1), "1234.5");

for (const locale of ["ar", "he", "fa", "ur", "ar-EG"]) {
  assert.equal(directionFor(locale), "rtl");
}
assert.equal(directionFor("en"), "ltr");
assert.equal(directionFor("zh"), "ltr");

console.log("i18n.test.ts ok");
