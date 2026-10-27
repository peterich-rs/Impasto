import assert from "node:assert/strict";
import {
  PrefixHeights,
  REFRESH_THRESHOLD,
  estimateHeight,
  nearEnd,
  pullDelta,
  retainHeights,
  stickyTranslate,
  thumbLength,
  thumbOffset,
  stabilizeWindow,
  windowRange,
  windowRangeForOffset,
} from "./list-window.ts";

const prefix = new PrefixHeights();
prefix.rebuild([10, 30]);
assert.equal(prefix.findIndex(0), 0);
assert.equal(prefix.findIndex(10), 1);
assert.equal(prefix.findIndex(100), 2);
assert.equal(prefix.itemStart(0), 0);
assert.equal(prefix.itemStart(1), 10);
assert.equal(prefix.total(), 40);

const alternating = new PrefixHeights();
const heights: number[] = [];
let naive = 0;
const naivePrefix = [0];
for (let index = 0; index < 1000; index += 1) {
  const height = index % 2 === 0 ? 12 : 28;
  heights.push(height);
  naive += height;
  naivePrefix.push(naive);
}
alternating.rebuild(heights);
for (let index = 0; index <= 1000; index += 17) {
  assert.equal(alternating.itemStart(index), naivePrefix[index]);
}
assert.equal(alternating.findIndex(naivePrefix[400] ?? 0), 400);

const reversed = [...heights].reverse();
alternating.rebuild(reversed);
assert.equal(alternating.heightAt(0), heights[999]);
assert.equal(alternating.itemStart(1), heights[999]);

alternating.add(0, 5);
assert.equal(alternating.heightAt(0), (heights[999] ?? 0) + 5);

const view = windowRange(prefix, 0, 10, 4);
assert.equal(view.start, 0);
assert.ok(view.end <= 2);

const held = windowRangeForOffset(prefix, 0, 10, 0, null);
assert.deepEqual(held, windowRange(prefix, 0, 10, 0));
const flung = windowRangeForOffset(prefix, 0, 10, 0, 40);
assert.ok(flung.end > held.end);

const rows = new PrefixHeights();
rows.rebuild(Array.from({ length: 80 }, () => 10));
const plain = windowRangeForOffset(rows, 100, 200, 4, null);
assert.deepEqual(plain, windowRange(rows, 100, 200, 4));
const led = windowRangeForOffset(rows, 100, 200, 4, 450);
assert.ok(led.end > plain.end);
const grown = stabilizeWindow(rows, plain, { ...plain, start: plain.start + 1 }, 4);
assert.equal(grown.start, plain.start);
const shifted = stabilizeWindow(rows, plain, { ...plain, start: plain.start + 5 }, 4);
assert.equal(shifted.start, plain.start + 5);
const expanded = stabilizeWindow(rows, plain, { ...plain, end: plain.end + 1 }, 4);
assert.equal(expanded.end, plain.end + 1);

const past = windowRange(prefix, 100, 10, 4);
assert.equal(past.start, 2);
assert.equal(past.end, 2);
assert.equal(past.top, 40);
assert.equal(past.bottom, 0);

assert.equal(estimateHeight(28), 28);
assert.equal(estimateHeight(0), 1);
assert.equal(estimateHeight(Number.NaN), 1);
assert.equal(estimateHeight(-4), 1);

assert.equal(stickyTranslate(null, 40, 20), 0);
assert.equal(stickyTranslate(100, 50, 30), 0);
assert.equal(stickyTranslate(80, 50, 40), -10);

assert.equal(pullDelta(0, -12, 0), 12);
assert.equal(pullDelta(8, -4, 0), 0);
assert.equal(pullDelta(0, 4, 4), 0);
assert.equal(REFRESH_THRESHOLD, 64);

assert.equal(thumbLength(100, 400), 25);
assert.equal(thumbOffset(100, 400, 300), 75);
assert.equal(thumbLength(40, 1000), 24);

assert.equal(nearEnd(96, 100, 4), false);
assert.equal(nearEnd(97, 100, 4), true);
assert.equal(nearEnd(100, 100, 4), true);

const heightsCache = new Map<string, number>();
for (let index = 0; index < 200; index += 1) {
  heightsCache.set(String(index), 10);
}
retainHeights(heightsCache, ["1", "2", "3"]);
assert.equal(heightsCache.size, 3);
assert.equal(heightsCache.has("1"), true);
assert.equal(heightsCache.has("9"), false);

console.log("list-window.test.ts ok");
