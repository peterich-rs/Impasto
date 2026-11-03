import assert from "node:assert/strict";
import { test } from "node:test";
import { SourceMap } from "./source-map.ts";

/**
 * Hand-built map: generated line 0 col 0 -> source 0 line 0 col 0, and
 * generated line 0 col 5 -> source 0 line 1 col 2. Encoded VLQ:
 * "A"=0, "K"=5, "C"=1, "E"=2 (value<<1, sign in bit 0).
 */
const MAP = JSON.stringify({
  sources: ["src/app.tsx"],
  sourcesContent: ["const a = 1;\nconst bb = 2;\n"],
  names: [],
  // Segment 1: [col 0, src 0, line 0, col 0] = AAAA
  // Segment 2: [col +5, src +0, line +1, col +2] = KACE
  mappings: "AAAA,KACE",
});

test("maps a known position back to the source", () => {
  const map = SourceMap.load(MAP);
  assert.ok(map);
  const first = map.original(0, 0);
  assert.deepEqual([first?.source, first?.line, first?.col], ["src/app.tsx", 0, 0]);
  const second = map.original(0, 6);
  assert.deepEqual([second?.source, second?.line, second?.col], ["src/app.tsx", 1, 2]);
});

test("a column before the first segment falls back to it", () => {
  const map = SourceMap.load(MAP);
  assert.ok(map);
  const position = map.original(0, 5);
  // col 5 is exactly segment 2's column.
  assert.equal(position?.line, 1);
});

test("a line with no mapping returns null", () => {
  const map = SourceMap.load(MAP);
  assert.ok(map);
  assert.equal(map.original(5, 0), null);
});

test("context shows the marked source line", () => {
  const map = SourceMap.load(MAP);
  assert.ok(map);
  const text = map.context("src/app.tsx", 1);
  assert.ok(text);
  assert.ok(text!.includes(">    2 const bb = 2;"));
});

test("bad json returns null", () => {
  assert.equal(SourceMap.load("{"), null);
  assert.equal(SourceMap.load("null"), null);
});

test("missing sourcesContent keeps context null", () => {
  const map = SourceMap.load(
    JSON.stringify({ sources: ["a.ts"], mappings: "AAAA", names: [] }),
  );
  assert.ok(map);
  assert.equal(map.context("a.ts", 0), null);
});
