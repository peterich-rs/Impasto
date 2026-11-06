import assert from "node:assert/strict";
import { installMockHost } from "../../packages/solid-ui/testing/renderTest.ts";
import { QuoteEngine } from "./src/engine/engine.ts";
import { QuoteBinder } from "./src/state/binder.ts";

const mock = installMockHost();
const engine = new QuoteEngine({ timeScale: 1, seed: 1 });
const binder = new QuoteBinder(
  engine,
  {
    up: [235, 62, 62, 255],
    down: [34, 176, 95, 255],
    flat: [138, 141, 145, 255],
    base: [30, 34, 40, 255],
  },
  () => "cn",
);

const symbol = engine.symbols()[0]!;
const last = mock.host.text("—", 14, [0, 0, 0, 255]);
const pct = mock.host.text("—", 12, [0, 0, 0, 255]);
const stopLast = binder.bind(symbol, { last, flashNode: last });
const stopPct = binder.bind(symbol, { pct, flashNode: pct });

const afterBind = mock.calls.filter((c) => c.name === "setText");
assert.equal(
  afterBind.some((c) => c.args[0] === last),
  true,
  "last cell written on bind",
);
assert.equal(
  afterBind.some((c) => c.args[0] === pct),
  true,
  "pct cell written on bind",
);

engine.runFor(5_000);
mock.calls.length = 0;
binder.flush();
const afterFlush = mock.calls.filter((c) => c.name === "setText");
assert.equal(
  afterFlush.some((c) => c.args[0] === last),
  true,
  "last cell still written after a sibling bind",
);
assert.equal(
  afterFlush.some((c) => c.args[0] === pct),
  true,
  "pct cell still written after a sibling bind",
);

stopLast();
engine.runFor(5_000);
mock.calls.length = 0;
binder.flush();
const afterUnbind = mock.calls.filter((c) => c.name === "setText");
assert.equal(
  afterUnbind.some((c) => c.args[0] === last),
  false,
  "unbound last cell is silent",
);
assert.equal(
  afterUnbind.some((c) => c.args[0] === pct),
  true,
  "remaining pct cell still writes",
);

stopPct();
const parent = mock.host.column(0);
const stale = mock.host.text("—", 14, [0, 0, 0, 255]);
mock.host.append(parent, stale);
binder.bind(symbol, { last: stale, flashNode: stale });
mock.host.remove(parent, stale);
assert.doesNotThrow(() => binder.flush());

const capEngine = new QuoteEngine({ timeScale: 1, seed: 2 });
const capBinder = new QuoteBinder(
  capEngine,
  {
    up: [235, 62, 62, 255],
    down: [34, 176, 95, 255],
    flat: [138, 141, 145, 255],
    base: [30, 34, 40, 255],
  },
  () => "cn",
);
const lastNodes: number[] = [];
for (const capSymbol of capEngine.symbols().slice(0, 10)) {
  const node = mock.host.text("—", 14, [0, 0, 0, 255]);
  capBinder.bind(capSymbol, { last: node });
  lastNodes.push(node);
}
capEngine.runFor(5_000);
mock.calls.length = 0;
capBinder.flush();
const firstFlush = mock.calls.filter((c) => c.name === "setText");
assert.equal(firstFlush.length <= 8, true, `first flush wrote ${firstFlush.length}`);
assert.equal(firstFlush.length > 0, true, "first flush wrote nothing");
mock.calls.length = 0;
capBinder.flush();
const secondFlush = mock.calls.filter((c) => c.name === "setText");
assert.equal(secondFlush.length > 0, true, "leftover cells wait for the next frame");
assert.equal(
  firstFlush.length + secondFlush.length <= lastNodes.length,
  true,
  "two flushes should not rewrite the first batch",
);

const pairEngine = new QuoteEngine({ timeScale: 1, seed: 3 });
const pairBinder = new QuoteBinder(
  pairEngine,
  {
    up: [235, 62, 62, 255],
    down: [34, 176, 95, 255],
    flat: [138, 141, 145, 255],
    base: [30, 34, 40, 255],
  },
  () => "cn",
);
const singles = pairEngine.symbols().slice(0, 7);
const pairSymbol = pairEngine.symbols()[7]!;
for (const capSymbol of singles) {
  pairBinder.bind(capSymbol, { last: mock.host.text("—", 14, [0, 0, 0, 255]) });
}
const pairLast = mock.host.text("—", 14, [0, 0, 0, 255]);
const pairPct = mock.host.text("—", 12, [0, 0, 0, 255]);
pairBinder.bind(pairSymbol, { last: pairLast, pct: pairPct });
pairEngine.runFor(5_000);
mock.calls.length = 0;
pairBinder.flush();
const pairFlush = mock.calls.filter((c) => c.name === "setText");
assert.equal(
  pairFlush.some((c) => c.args[0] === pairLast || c.args[0] === pairPct),
  false,
  "a last+pct bind must not split across the cap",
);
mock.calls.length = 0;
pairBinder.flush();
const pairNext = mock.calls.filter((c) => c.name === "setText");
assert.equal(
  pairNext.some((c) => c.args[0] === pairLast) && pairNext.some((c) => c.args[0] === pairPct),
  true,
  "the deferred pair writes together",
);

console.log("quotes binder ok");
