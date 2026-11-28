import { assert, test } from "impasto:test";
import { compareStrings, formatDateTime, formatNumber, formatRelativeTime, parseLocale, pluralCategory } from "../src/index";

test("ICU4X formats Arabic plurals, IANA time zones and numeric collation", () => {
  assert.deepEqual([0, 1, 2, 3, 11, 100].map(value => pluralCategory("ar", value)),
    ["zero", "one", "two", "few", "many", "other"]);
  assert.equal(parseLocale("ar-EG").direction, "rtl");
  assert.equal(compareStrings("en", "item 2", "item 10", { numeric: true }), -1);
  assert.equal(formatRelativeTime("en", -1, { unit: "day", numeric: "auto" }), "yesterday");
  assert.equal(formatNumber("en", 1_200, { notation: "compactShort" }), "1.2K");
  assert.equal(formatDateTime("en", Date.UTC(2025, 0, 1, 0, 0), {
    timeStyle: "short", timeZone: "Asia/Shanghai", hour12: false,
  }), "08:00");
});
