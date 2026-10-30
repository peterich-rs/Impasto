import assert from "node:assert/strict";
import { formatDate, formatNumber, plural } from "./index.ts";

assert.equal(plural("en", 1, { one: "item", other: "items" }), "item");
assert.equal(plural("en", 2, { one: "item", other: "items" }), "items");
assert.equal(plural("zh", 1, { one: "item", other: "items" }), "items");

assert.equal(formatNumber("en", 1.25, 1), "1.3");
assert.equal(formatNumber("en", -1.25, 1), "-1.3");
assert.equal(formatNumber("en", 1234.5, 1), "1,234.5");
assert.equal(formatNumber("en", Number.NaN, 1), "");
assert.equal(formatNumber("zh", 1234.5, 1), "1234.5");

const formatted = formatDate("en", Date.UTC(2024, 0, 2, 3, 4));
assert.match(formatted, /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);

console.log("i18n.test.ts ok");
