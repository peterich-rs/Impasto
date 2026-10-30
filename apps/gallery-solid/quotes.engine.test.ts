import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { mulberry32, hashString, gauss } from "./src/engine/rng.ts";
import { QuoteEngine } from "./src/engine/engine.ts";
import { TickRing } from "./src/engine/types.ts";

// Determinism: same seed, identical tick streams.
{
  const a = new QuoteEngine({ timeScale: 1, seed: 7 });
  const b = new QuoteEngine({ timeScale: 1, seed: 7 });
  a.runFor(60_000);
  b.runFor(60_000);
  for (const symbol of a.symbols()) {
    const ra = a.ring(symbol);
    const rb = b.ring(symbol);
    assert.equal(ra.length, rb.length, symbol);
    for (let i = 0; i < ra.length; i++) {
      assert.equal(ra.at(i).price, rb.at(i).price, `${symbol} tick ${i}`);
    }
  }
}

// Different seeds diverge.
{
  const a = new QuoteEngine({ timeScale: 1, seed: 7 });
  const b = new QuoteEngine({ timeScale: 1, seed: 8 });
  a.runFor(60_000);
  b.runFor(60_000);
  const diff = a.symbols().some((s) => a.quote(s).last !== b.quote(s).last);
  assert.ok(diff, "seeds should diverge");
}

// cn price limits hold.
{
  const e = new QuoteEngine({ timeScale: 1, seed: 3 });
  e.runFor(600_000);
  for (const symbol of e.symbols()) {
    const q = e.quote(symbol);
    if (symbol.endsWith(".CN")) {
      assert.ok(q.last <= q.prevClose * 1.1001, `${symbol} ${q.last} > ${q.prevClose * 1.1}`);
      assert.ok(q.last >= q.prevClose * 0.8999, `${symbol} ${q.last} < ${q.prevClose * 0.9}`);
    }
    assert.ok(q.high >= q.low, symbol);
    assert.ok(q.volume > 0, symbol);
  }
}

// Day settle and reopen keeps bars growing and ring fresh.
{
  // Short trading day keeps the tick loop small while exercising settle.
  const e = new QuoteEngine({ timeScale: 1, seed: 5, dayMs: 90_000 });
  const before = e.bars("AAPL.US").length;
  e.runFor(90_000 + 2000);
  const after = e.bars("AAPL.US").length;
  assert.ok(after > before, `bars ${before} -> ${after}`);
  const q = e.quote("AAPL.US");
  assert.equal(q.prevClose !== 0, true);
  assert.ok(e.ring("AAPL.US").length <= 240);
}

// Ring buffer wraps without shifting.
{
  const ring = new TickRing(8);
  for (let i = 0; i < 20; i++) {
    ring.push({ t: i, price: i, volume: 0 });
  }
  assert.equal(ring.length, 8);
  assert.equal(ring.at(0).price, 12);
  assert.equal(ring.at(7).price, 19);
}

// Step scaling: real dt maps to sim dt.
{
  const e = new QuoteEngine({ timeScale: 60, seed: 1 });
  e.step(1000);
  assert.equal(e.simNow(), 60_000);
}

// mulberry32 reproducibility and gauss spare usage.
{
  const r1 = mulberry32(42);
  const r2 = mulberry32(42);
  for (let i = 0; i < 10; i++) {
    assert.equal(r1(), r2());
  }
  const st = { spare: null };
  const vals = [gauss(r1, st), gauss(r1, st), gauss(r1, st)];
  assert.equal(vals.every((v) => Number.isFinite(v)), true);
  assert.equal(hashString("00700.HK") > 0, true);
}


console.log("engine tests ok");
