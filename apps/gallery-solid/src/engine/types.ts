/** Data shapes shared by the engine, the trading store, and the UI. */

export type Market = "hk" | "us" | "cn";

export type Instrument = {
  symbol: string;
  name: string;
  market: Market;
  /** Anchor price for generated history. */
  base: number;
  /** Annualized volatility, 0.20–0.60. */
  sigma: number;
  /** Annualized drift, small. */
  drift: number;
  /** Monogram background. */
  brand: string;
  /** Order lot size: 100 for cn, 1 otherwise. */
  lot: number;
  index?: boolean;
};

export type Bar = { t: number; o: number; h: number; l: number; c: number; v: number };

export type Tick = { t: number; price: number; volume: number };

export type Quote = {
  symbol: string;
  last: number;
  prevClose: number;
  open: number;
  high: number;
  low: number;
  volume: number;
  /** Simulated ms of the current trading day start. */
  dayStart: number;
  /** Bumped on every tick. The write layer skips rows whose revision is stale. */
  revision: number;
};

export type OrderSide = "buy" | "sell";
export type OrderType = "market" | "limit";
export type OrderStatus = "pending" | "filled" | "cancelled" | "rejected";

export type Order = {
  id: string;
  symbol: string;
  side: OrderSide;
  type: OrderType;
  price: number;
  qty: number;
  filled: number;
  avgFill: number;
  status: OrderStatus;
  createdAt: number;
};

export type Position = { symbol: string; qty: number; avgCost: number };

/** Fixed-capacity ring buffer of ticks. Push is O(1); no array shifting. */
export class TickRing {
  readonly cap: number;
  readonly t: Float64Array;
  readonly p: Float64Array;
  readonly v: Float64Array;
  #head = 0;
  #len = 0;

  constructor(capacity = 240) {
    this.cap = capacity;
    this.t = new Float64Array(capacity);
    this.p = new Float64Array(capacity);
    this.v = new Float64Array(capacity);
  }

  get length(): number {
    return this.#len;
  }

  push(tick: Tick): void {
    const at = (this.#head + this.#len) % this.cap;
    this.t[at] = tick.t;
    this.p[at] = tick.price;
    this.v[at] = tick.volume;
    if (this.#len < this.cap) {
      this.#len += 1;
    } else {
      this.#head = (this.#head + 1) % this.cap;
    }
  }

  /** `i === 0` is the oldest retained tick. */
  at(i: number): Tick {
    if (i < 0 || i >= this.#len) {
      throw new Error(`tick ${i} of ${this.#len}`);
    }
    const at = (this.#head + i) % this.cap;
    return { t: this.t[at], price: this.p[at], volume: this.v[at] };
  }

  clear(): void {
    this.#head = 0;
    this.#len = 0;
  }
}
