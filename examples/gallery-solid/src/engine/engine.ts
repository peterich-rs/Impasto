import { gauss, hashString, mulberry32, type GaussState } from "./rng.ts";
import { UNIVERSE, clampLimits, dailyHistory, round2 } from "./universe.ts";
import { TickRing, type Bar, type Instrument, type Quote } from "./types.ts";
import { Analytics, minute } from "./analytics.ts";
const DAY_MS = 24 * 3600 * 1000;
/** Each engine tick represents one second of market time. */
const TICK_SIM_MS = 1000;
type Sym = {
  ins: Instrument;
  quote: Quote;
  ring: TickRing;
  bars: Bar[];
  analytics: Analytics;
  rand: () => number;
  st: GaussState;
  nextTickAt: number;
  interval: number;
};
export type TickListener = (symbol: string, price: number, t: number) => void;

/**
 * Deterministic mock market. `step` advances the simulated clock; symbols whose
 * `nextTickAt` has passed emit one tick each. The provider schedules steps.
 */
export class QuoteEngine {
  readonly #syms = new Map<string, Sym>();
  readonly #listeners = new Set<TickListener>();
  #simNow = 0;
  #timeScale: number;
  #dayMs: number;
  #nextDay: number;
  #orderSeq = 0;
  constructor(
    opts: {
      timeScale?: number;
      seed?: number;
      dayMs?: number;
      warmupMinutes?: number;
    } = {},
  ) {
    this.#timeScale = opts.timeScale ?? 60;
    this.#dayMs = opts.dayMs ?? DAY_MS;
    const warmup = opts.warmupMinutes ?? 0;
    const start = -warmup * minute;
    this.#nextDay = this.#dayMs + start;
    const master = opts.seed ?? 0;
    for (const ins of UNIVERSE) {
      const rand = mulberry32((master ^ hashString(ins.symbol)) >>> 0);
      const st: GaussState = {
        spare: null,
      };
      const bars = dailyHistory(ins, 250);
      const prevClose = round2(bars[bars.length - 1]!.c);
      const open = clampLimits(
        prevClose * (1 + ins.sigma * 0.006 * gauss(rand, st)),
        prevClose,
        ins,
      );
      const q: Quote = {
        symbol: ins.symbol,
        last: open,
        prevClose,
        open,
        high: open,
        low: open,
        volume: 0,
        dayStart: start,
        revision: 1,
      };
      const interval = 100 + Math.floor(rand() * 400);
      const s: Sym = {
        ins,
        quote: q,
        ring: new TickRing(240),
        bars,
        analytics: new Analytics(),
        rand,
        st,
        nextTickAt: Math.floor(rand() * interval),
        interval,
      };
      this.#record(s, start, open, 0);
      for (let index = 1; index <= warmup; index += 1) {
        for (let part = 0; part < 4; part += 1) {
          const t = start + index * minute - (3 - part) * minute / 4;
          this.#trade(s, t, minute / 4);
        }
      }
      this.#syms.set(ins.symbol, s);
    }
  }
  symbols(): string[] {
    return [...this.#syms.keys()];
  }
  instruments(): Instrument[] {
    return [...this.#syms.values()].map((s) => s.ins);
  }
  ins(symbol: string): Instrument {
    return this.#sym(symbol).ins;
  }
  quote(symbol: string): Quote {
    return this.#sym(symbol).quote;
  }
  ring(symbol: string): TickRing {
    return this.#sym(symbol).ring;
  }
  analytics(symbol: string): Analytics {
    return this.#sym(symbol).analytics;
  }
  bars(symbol: string): readonly Bar[] {
    return this.#sym(symbol).bars;
  }
  simNow(): number {
    return this.#simNow;
  }
  dayStart(): number {
    return this.#nextDay - this.#dayMs;
  }
  setTimeScale(scale: number): void {
    if (Number.isFinite(scale) && scale > 0) {
      this.#timeScale = scale;
    }
  }
  nextId(prefix: string): string {
    this.#orderSeq += 1;
    return `${prefix}-${this.#orderSeq}`;
  }
  onTick(fn: TickListener): () => void {
    this.#listeners.add(fn);
    return () => {
      this.#listeners.delete(fn);
    };
  }

  /** Advances the simulated clock by `realDtMs * timeScale`. Returns true if any tick fired. */
  step(realDtMs: number): boolean {
    this.#simNow += realDtMs * this.#timeScale;
    return this.#drain();
  }

  /** Advances the clock directly in simulated ms. Tests use this for determinism. */
  runFor(simMs: number): void {
    let remaining = simMs;
    while (remaining > 0) {
      this.#simNow += TICK_SIM_MS;
      remaining -= TICK_SIM_MS;
      this.#drain();
    }
  }
  #drain(): boolean {
    let ticked = false;
    for (const s of this.#syms.values()) {
      let guard = 0;
      while (s.nextTickAt <= this.#simNow && guard < 64) {
        guard += 1;
        this.#advanceOne(s);
        s.nextTickAt = this.#simNow + Math.floor(s.rand() * s.interval) + 50;
        ticked = true;
      }
    }
    if (this.#simNow >= this.#nextDay) {
      this.#settleAndReopen();
      ticked = true;
    }
    return ticked;
  }
  #advanceOne(s: Sym): void {
    this.#trade(s, this.#simNow, TICK_SIM_MS);
    for (const fn of this.#listeners) {
      fn(s.ins.symbol, s.quote.last, this.#simNow);
    }
  }
  #trade(s: Sym, t: number, dt: number): void {
    const dtYear = dt / (6.5 * 3_600_000 * 252);
    const z = gauss(s.rand, s.st);
    const next = round2(
      clampLimits(
        s.quote.last *
          Math.exp(
            (s.ins.drift - (s.ins.sigma * s.ins.sigma) / 2) * dtYear +
              s.ins.sigma * Math.sqrt(dtYear) * z,
          ),
        s.quote.prevClose,
        s.ins,
      ),
    );
    const sizeRoll = s.rand();
    const volume = Math.ceil((sizeRoll > 0.98 ? 80 + sizeRoll * 80 : 1 + sizeRoll ** 3 * 55)) * Math.max(100, s.ins.lot);
    const side = next > s.quote.last || (next === s.quote.last && sizeRoll < 0.5) ? "buy" : "sell";
    this.#record(s, t, next, volume, side);
  }
  #record(s: Sym, t: number, price: number, volume: number, side?: "buy" | "sell"): void {
    const tick = { t, price, volume, side };
    s.quote.last = price;
    s.quote.high = Math.max(s.quote.high, price);
    s.quote.low = Math.min(s.quote.low, price);
    s.quote.volume += volume;
    s.quote.revision += 1;
    s.ring.push(tick);
    s.analytics.add(tick, s.quote.dayStart);
  }
  #settleAndReopen(): void {
    for (const s of this.#syms.values()) {
      const q = s.quote;
      s.bars.push({
        t: q.dayStart,
        o: q.open,
        h: q.high,
        l: q.low,
        c: q.last,
        v: q.volume,
      });
      if (s.bars.length > 400) {
        s.bars.shift();
      }
      const prevClose = q.last;
      const open = clampLimits(
        prevClose * (1 + s.ins.sigma * 0.006 * gauss(s.rand, s.st)),
        prevClose,
        s.ins,
      );
      s.ring.clear();
      s.analytics.reset();
      q.prevClose = prevClose;
      q.open = open;
      q.last = open;
      q.high = open;
      q.low = open;
      q.volume = 0;
      q.dayStart = this.#nextDay;
      this.#record(s, q.dayStart, open, 0);
      s.nextTickAt = this.#simNow + Math.floor(s.rand() * s.interval);
      for (const fn of this.#listeners) {
        fn(s.ins.symbol, open, this.#simNow);
      }
    }
    this.#nextDay += this.#dayMs;
  }
  #sym(symbol: string): Sym {
    const s = this.#syms.get(symbol);
    if (s === undefined) {
      throw new Error(`unknown symbol ${symbol}`);
    }
    return s;
  }
}
