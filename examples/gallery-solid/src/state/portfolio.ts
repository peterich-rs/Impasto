import type { Position } from "../engine/types.ts";

/**
 * Opening book for a fresh install. Costs sit below and above the engine
 * anchor prices so the first screen shows a realistic mix of gains and
 * losses across three markets. Sizes are in board lots.
 */
export const SEED_POSITIONS: readonly Position[] = [
  { symbol: "00700.HK", qty: 4_000, avgCost: 342.6 },
  { symbol: "AAPL.US", qty: 800, avgCost: 226.4 },
  { symbol: "NVDA.US", qty: 600, avgCost: 104.8 },
  { symbol: "600519.CN", qty: 300, avgCost: 1_536.0 },
  { symbol: "300750.CN", qty: 1_500, avgCost: 168.5 },
  { symbol: "09988.HK", qty: 6_000, avgCost: 79.3 },
  { symbol: "01810.HK", qty: 12_000, avgCost: 15.9 },
  { symbol: "TSLA.US", qty: 300, avgCost: 258.0 },
];

export const SEED_CASH = 280_000;
