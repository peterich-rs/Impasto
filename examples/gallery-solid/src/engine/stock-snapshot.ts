import { book, type MinuteBar } from "./analytics.ts";
import type { QuoteEngine } from "./engine.ts";
import type { Bar, Quote, Tick } from "./types.ts";

export const tapeRows = 14;

export type StockSnapshot = {
  quote: Quote;
  history: readonly Bar[];
  minutes: readonly MinuteBar[];
  trades: readonly Tick[];
  turnover: number;
  flow: { buy: number[]; sell: number[] };
  readonly book: ReturnType<typeof book>;
};

export function stockSnapshot(engine: QuoteEngine, symbol: string, previous?: StockSnapshot): StockSnapshot {
  const live = engine.quote(symbol);
  if (previous?.quote.symbol === symbol && previous.quote.revision === live.revision) return previous;
  const quote = { ...live };
  const analytics = engine.analytics(symbol);
  const minutes = analytics.minutes.slice();
  // Only the open minute is mutated in place by subsequent simulation ticks.
  const last = minutes.at(-1);
  if (last) minutes[minutes.length - 1] = { ...last };
  const ring = engine.ring(symbol);
  const trades: Tick[] = [];
  for (let index = ring.length - 1; index >= Math.max(0, ring.length - tapeRows); index--) {
    const tick = ring.at(index);
    if (tick.side) trades.push(tick);
  }
  const history = previous?.quote.symbol === symbol && previous.quote.dayStart === quote.dayStart
    ? previous.history : engine.bars(symbol).slice();
  let levels: ReturnType<typeof book> | undefined;
  return {
    quote, history, minutes, trades,
    turnover: analytics.turnover,
    flow: { buy: analytics.buy.slice(), sell: analytics.sell.slice() },
    get book() {
      return levels ??= book(quote, trades[0]?.side, engine.ins(symbol).lot);
    },
  };
}
