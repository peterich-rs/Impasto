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

console.log("quotes binder ok");
