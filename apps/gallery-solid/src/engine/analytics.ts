import { hashString, mulberry32 } from "./rng.ts";
import type { Bar, Instrument, OrderSide, Quote, Tick } from "./types.ts";

export const minute = 60_000;
export const marketOpen = 9 * 60 + 30;
export const blockSize = 8_000;
export type Period = "intraday" | "1m" | "5m" | "15m" | "1D";
export type MinuteBar = Bar & { turnover: number; avg: number };
export type Level = { price: number; size: number; total: number };

export class Analytics {
  readonly minutes: MinuteBar[] = [];
  readonly buy = [0, 0, 0, 0];
  readonly sell = [0, 0, 0, 0];
  turnover = 0;
  volume = 0;

  add(tick: Tick, start: number): void {
    const t = Math.floor((tick.t - start) / minute) * minute;
    const { price, volume, side } = tick;
    const amount = price * volume;
    this.turnover += amount;
    this.volume += volume;
    if (side) {
      const size = volume >= blockSize ? 3 : volume > 2_000 ? 2 : volume > 500 ? 1 : 0;
      this[side][size]! += volume;
    }
    const avg = this.volume === 0 ? price : this.turnover / this.volume;
    const last = this.minutes.at(-1);
    if (last?.t === t) {
      last.c = price;
      last.h = Math.max(last.h, price);
      last.l = Math.min(last.l, price);
      last.v += volume;
      last.turnover += amount;
      last.avg = avg;
    } else {
      this.minutes.push({ t, o: price, h: price, l: price, c: price, v: volume, turnover: amount, avg });
      if (this.minutes.length > 1_440) this.minutes.shift();
    }
  }

  reset(): void {
    this.minutes.length = 0;
    this.buy.fill(0);
    this.sell.fill(0);
    this.turnover = this.volume = 0;
  }
}

export function series(period: Period, source: readonly MinuteBar[], history: readonly Bar[], quote: Quote): Bar[] {
  if (period === "1D") {
    return [...history.slice(-160), { t: quote.dayStart, o: quote.open, h: quote.high, l: quote.low, c: quote.last, v: quote.volume }];
  }
  if (period === "intraday" || period === "1m") return source.slice();
  const interval = (period === "5m" ? 5 : 15) * minute;
  const bars: Bar[] = [];
  for (const bar of source) {
    const t = Math.floor(bar.t / interval) * interval;
    const last = bars.at(-1);
    if (last?.t === t) {
      last.h = Math.max(last.h, bar.h);
      last.l = Math.min(last.l, bar.l);
      last.c = bar.c;
      last.v += bar.v;
    } else {
      bars.push({ t, o: bar.o, h: bar.h, l: bar.l, c: bar.c, v: bar.v });
    }
  }
  return bars;
}

export function book(quote: Quote, lastSide: OrderSide | undefined, lot: number) {
  const random = mulberry32(hashString(quote.symbol) ^ quote.revision);
  const cents = Math.round(quote.last * 100);
  const bestBid = Math.max(11, cents - (lastSide === "sell" ? 0 : 1));
  const levels = (first: number, direction: number) => {
    let total = 0;
    return Array.from({ length: 12 }, (_, index): Level => {
      const size = Math.round((1 + random() ** 2 * 70) * Math.max(100, lot));
      total += size;
      return { price: (first + direction * index) / 100, size, total };
    });
  };
  return { bids: levels(bestBid, -1), asks: levels(bestBid + 1, 1), mid: (bestBid + 0.5) / 100 };
}

export const sectors = ["technology", "health", "finance", "consumer", "industrials", "energy"] as const;
export type Sector = typeof sectors[number];
const sectorByCode: Readonly<Record<string, Sector>> = {
  "00700": "technology", "00981": "technology", "01810": "technology", "09988": "technology",
  AAPL: "technology", NVDA: "technology", MSFT: "technology", META: "technology", GOOGL: "technology",
  "688981": "technology", "002415": "technology", "03690": "consumer", "09618": "consumer",
  AMZN: "consumer", BILI: "consumer", PDD: "consumer", "600519": "consumer", "000858": "consumer",
  "00005": "finance", "00939": "finance", "02318": "finance", "601318": "finance", "600036": "finance", "300059": "finance",
  TSLA: "industrials", "002594": "industrials", "01211": "industrials", "300750": "industrials",
  "00883": "energy", "601899": "industrials",
};

export function sectorOf(instrument: Instrument): Sector | undefined {
  return sectorByCode[instrument.symbol.split(".")[0]!];
}
