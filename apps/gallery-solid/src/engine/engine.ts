import { gauss, hashString, mulberry32, type GaussState } from "./rng.ts";
import {
  UNIVERSE,
  clampLimits,
  dailyHistory,
  round2,
} from "./universe.ts";
import { TickRing, type Bar, type Instrument, type Quote } from "./types.ts";

const DAY_MS = 24 * 3600 * 1000;
/** Each engine tick represents one second of market time. */
const TICK_SIM_MS = 1000;

type Sym = {
  ins: Instrument;
  quote: Quote;
  ring: TickRing;
  bars: Bar[];
  rand: () => number;
  st: GaussState;
  nextTickAt: number;
  interval: number;
};

export type TickListener = (symbol: string, price: number, t: number) => void;

/**
 * Deterministic mock market. `step` advances the simulated clock; symbols whose
 * `nextTickAt` has passed emit one tick each. No timers: the provider drives
 * this from `onFrame`.
 */
export class QuoteEngine {
  readonly #syms = new Map<string, Sym>();
  readonly #listeners = new Set<TickListener>();
  #simNow = 0;
  #timeScale: number;
  #dayMs: number;
  #orderSeq = 0;

  constructor(opts: { timeScale?: number; seed?: number; dayMs?: number } = {}) {
    this.#timeScale = opts.timeScale ?? 60;
    this.#dayMs = opts.dayMs ?? DAY_MS;
    const master = opts.seed ?? 0;
    for (const ins of UNIVERSE) {
      const rand = mulberry32((master ^ hashString(ins.symbol)) >>> 0);
      const st: GaussState = { spare: null };
      const bars = dailyHistory(ins, 250);
      const prevClose = round2(bars[bars.length - 1]!.c);
      const open = clampLimits(prevClose * (1 + ins.sigma * 0.006 * gauss(rand, st)), prevClose, ins);
      const q: Quote = {
        symbol: ins.symbol,
        last: open,
        prevClose,
        open,
        high: open,
        low: open,
        volume: 0,
        dayStart: 0,
        revision: 1,
      };
      const interval = 100 + Math.floor(rand() * 400);
      const s: Sym = {
        ins,
        quote: q,
        ring: new TickRing(240),
        bars,
        rand,
        st,
        nextTickAt: Math.floor(rand() * interval),
        interval,
      };
      s.ring.push({ t: 0, price: open, volume: 0 });
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

  bars(symbol: string): readonly Bar[] {
    return this.#sym(symbol).bars;
  }

  simNow(): number {
    return this.#simNow;
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
    if (this.#simNow >= this.#dayMs) {
      this.#settleAndReopen();
    }
    return ticked;
  }

  #advanceOne(s: Sym): void {
    const dtYear = TICK_SIM_MS / (1000 * 3600 * 24 * 252);
    const z = gauss(s.rand, s.st);
    const next = round2(
      clampLimits(
        s.quote.last * Math.exp((s.ins.drift - (s.ins.sigma * s.ins.sigma) / 2) * dtYear + s.ins.sigma * Math.sqrt(dtYear) * z),
        s.quote.prevClose,
        s.ins,
      ),
    );
    s.quote.last = next;
    s.quote.high = Math.max(s.quote.high, next);
    s.quote.low = Math.min(s.quote.low, next);
    const lot = s.ins.lot === 1 ? 1 : 100;
    const volume = Math.floor((30 + s.rand() * 160) * lot * (s.ins.index ? 40 : 1));
    s.quote.volume += volume;
    s.ring.push({ t: this.#simNow, price: next, volume });
    s.quote.revision += 1;
    for (const fn of this.#listeners) {
      fn(s.ins.symbol, next, this.#simNow);
    }
  }

  #settleAndReopen(): void {
    for (const s of this.#syms.values()) {
      const q = s.quote;
      s.bars.push({ t: q.dayStart, o: q.open, h: q.high, l: q.low, c: q.last, v: q.volume });
      if (s.bars.length > 400) {
        s.bars.shift();
      }
      const prevClose = q.last;
      const open = clampLimits(prevClose * (1 + s.ins.sigma * 0.006 * gauss(s.rand, s.st)), prevClose, s.ins);
      s.ring.clear();
      s.ring.push({ t: 0, price: open, volume: 0 });
      q.prevClose = prevClose;
      q.open = open;
      q.last = open;
      q.high = open;
      q.low = open;
      q.volume = 0;
      q.dayStart += this.#dayMs;
      q.revision += 1;
      s.nextTickAt = this.#simNow + Math.floor(s.rand() * s.interval);
      for (const fn of this.#listeners) {
        fn(s.ins.symbol, open, this.#simNow);
      }
    }
    this.#simNow -= DAY_MS;
  }

  #sym(symbol: string): Sym {
    const s = this.#syms.get(symbol);
    if (s === undefined) {
      throw new Error(`unknown symbol ${symbol}`);
    }
    return s;
  }
}
