import { QuoteEngine } from "../../apps/gallery-solid/src/engine/engine.ts";
import { stockSnapshot } from "../../apps/gallery-solid/src/engine/stock-snapshot.ts";
import { downsampleTicks } from "../../apps/gallery-solid/src/engine/universe.ts";

const engine = new QuoteEngine({ timeScale: 60, warmupMinutes: 50, dayMs: 6.5 * 3_600_000 });
const instruments = engine.instruments().filter((instrument) => !instrument.index);
const symbol = "00700.HK";
const daily = engine.bars(symbol).slice(-90);
const frames = Array.from({ length: 75 }, (_, index) => {
  if (index > 0) {
    for (let tick = 0; tick < 6; tick++) engine.step(1000 / 30);
  }
  const detail = stockSnapshot(engine, symbol);
  return {
    rows: instruments.map(({ symbol }) => {
      const quote = engine.quote(symbol);
      return [quote.last, (quote.last / quote.prevClose - 1) * 100, downsampleTicks(engine.ring(symbol), 24)];
    }),
    detail: {
      minute: detail.minutes.map((bar) => bar.c),
      quote: detail.quote,
      book: detail.book,
      trades: detail.trades,
    },
  };
});

process.stdout.write(JSON.stringify({
  instruments: instruments.map(({ symbol, name }) => ({ symbol, name })),
  detailIndex: instruments.findIndex((item) => item.symbol === symbol),
  daily: daily.map((bar) => [bar.o, bar.h, bar.l, bar.c]),
  periodMillis: 200,
  frames,
}));
