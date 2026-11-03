import assert from "node:assert/strict";
import { test } from "node:test";
import { appendCapped } from "./cursor.ts";

test("append keeps the tail and ignores an empty page", () => {
  assert.deepEqual(appendCapped([1, 2], [], 4), [1, 2]);
  assert.deepEqual(appendCapped([1, 2], [3, 4, 5], 4), [2, 3, 4, 5]);
});

test("the first page is not copied when it already fits", () => {
  const page = [1, 2];
  assert.equal(appendCapped([], page, 4), page);
});
