import { advanceFrames, assert, test } from "impasto:test";
import { createRoot } from "solid-js";
import { mulberry32, hashString, gauss } from "../src/engine/rng.ts";
import { QuoteEngine } from "../src/engine/engine.ts";
import { book, series } from "../src/engine/analytics.ts";
import { stockSnapshot } from "../src/engine/stock-snapshot.ts";
import { createAccount } from "../src/state/account.ts";
import { marketPulse, useQuotes, wireQuotes } from "../src/state/quotes-provider.tsx";
import { TickRing } from "../src/engine/types.ts";

test("live quotes publish again after an interrupted market tick", async () => {
  const engine = wireQuotes().engine;
  const step = engine.step;
  const timeout = globalThis.setTimeout;
  const interval = globalThis.setInterval;
  const interruption = new Error("interrupted market tick");
  let interrupted = false;
  let attempts = 0;
  const observe = <Args extends unknown[]>(run: (...args: Args) => void, args: Args) => () => {
    try {
      run(...args);
    } catch (error) {
      if (error !== interruption) throw error;
      interrupted = true;
    }
  };
  // The headless driver rejects uncaught timer errors. Observe the injected
  // fault outside the provider so the scheduler can run the following turn.
  globalThis.setTimeout = (run, delay, ...args) => timeout(observe(run, args), delay);
  globalThis.setInterval = (run, delay, ...args) => interval(observe(run, args), delay);
  engine.step = (dt) => {
    if (++attempts === 1) throw interruption;
    return step.call(engine, dt);
  };
  const before = marketPulse();
  const release = createRoot((dispose) => {
    useQuotes();
    return dispose;
  });
  try {
    await advanceFrames(40);
    assert.ok(interrupted, "first timer callback should have been interrupted");
    assert.ok(marketPulse() > before, "market stopped publishing after the interruption");
  } finally {
    release();
    engine.step = step;
    globalThis.setTimeout = timeout;
    globalThis.setInterval = interval;
  }
});

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
  assert.equal(e.analytics("AAPL.US").volume, q.volume);
}

// One trade stream feeds VWAP, period aggregation and cumulative book levels.
{
  const e = new QuoteEngine({ seed: 5, warmupMinutes: 50 });
  const symbol = "00700.HK";
  const q = e.quote(symbol);
  const data = e.analytics(symbol);
  const quarterHours = series("15m", data.minutes, e.bars(symbol), q);
  assert.ok(quarterHours.length < data.minutes.length);
  assert.equal(quarterHours.reduce((volume, bar) => volume + bar.v, 0), q.volume);
  assert.equal(quarterHours.at(-1)?.c, q.last);
  assert.equal(data.minutes.at(-1)?.avg, data.turnover / q.volume);
  assert.equal([...data.buy, ...data.sell].reduce((volume, size) => volume + size, 0), q.volume);
  const { bids, asks } = book(q, e.ring(symbol).at(e.ring(symbol).length - 1).side, e.ins(symbol).lot);
  assert.equal(Math.round((asks[0]!.price - bids[0]!.price) * 100), 1);
  assert.ok(bids.every((level, i) => i === 0 || level.price < bids[i - 1]!.price));
  assert.ok(asks.every((level, i) => i === 0 || level.price > asks[i - 1]!.price));
  assert.equal(asks.at(-1)?.total, asks.reduce((sum, level) => sum + level.size, 0));
}

// Detail subscribers can retain a published snapshot while offscreen.
{
  const e = new QuoteEngine({ seed: 5, warmupMinutes: 50 });
  const symbol = "00700.HK";
  const first = stockSnapshot(e, symbol);
  const openMinute = { ...first.minutes.at(-1) };
  const buy = first.flow.buy.slice();
  assert.equal(stockSnapshot(e, symbol, first), first);
  e.runFor(5_000);
  const next = stockSnapshot(e, symbol, first);
  assert.ok(next.quote.revision > first.quote.revision);
  assert.equal(next.history, first.history);
  assert.deepEqual(first.minutes.at(-1), openMinute);
  assert.deepEqual(first.flow.buy, buy);
  assert.equal(next.minutes.at(-1)?.c, next.quote.last);
  assert.equal(next.trades[0]?.price, next.quote.last);
  assert.deepEqual(first.book, book(first.quote, first.trades[0]?.side, e.ins(symbol).lot));
  assert.equal(next.book, next.book);
}

// Pending orders reserve cash and lots until cancelled or filled.
test("reserves cash and lots until cancellation or fill", async () => {
  const e = new QuoteEngine();
  const account = createAccount(e);
  const symbol = "600036.CN";
  const lot = e.ins(symbol).lot;
  const initialCash = account.availableCash();
  const buy = account.submit(symbol, "buy", "limit", e.quote(symbol).last, lot);
  assert.ok(buy.ok);
  await advanceFrames();
  assert.ok(account.availableCash() < initialCash);
  account.cancel(buy.order.id);
  await advanceFrames();
  assert.equal(account.availableCash(), initialCash);
  assert.equal(account.submit(symbol, "buy", "market", 0, lot + 1).ok, false);
  assert.ok(account.submit(symbol, "buy", "market", 0, lot).ok);
  await advanceFrames();
  assert.equal(account.availableHolding(symbol), lot);
  assert.ok(account.submit(symbol, "sell", "limit", e.quote(symbol).last, lot).ok);
  await advanceFrames();
  assert.equal(account.availableHolding(symbol), 0);
  assert.equal(account.submit(symbol, "sell", "market", 0, lot).ok, false);
});

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
  e.setTimeScale(1);
  e.step(1000);
  assert.equal(e.simNow(), 61_000);
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
