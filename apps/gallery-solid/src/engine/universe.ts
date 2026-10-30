import type { Bar, Instrument } from "./types.ts";
import { gauss, mulberry32, type GaussState } from "./rng.ts";

/** Three markets, indices first. Names evoke real tickers without logos. */
export const UNIVERSE: Instrument[] = [
  { symbol: "HSI.HK", name: "恒生指数", market: "hk", base: 19850, sigma: 0.18, drift: 0.03, brand: "#8A2BE2", lot: 1, index: true },
  { symbol: "HSTECH.HK", name: "恒生科技", market: "hk", base: 4120, sigma: 0.26, drift: 0.05, brand: "#1E90FF", lot: 1, index: true },
  { symbol: "SPX.US", name: "标普500", market: "us", base: 5470, sigma: 0.15, drift: 0.08, brand: "#2E8B57", lot: 1, index: true },
  { symbol: "NDX.US", name: "纳斯达克100", market: "us", base: 19870, sigma: 0.22, drift: 0.12, brand: "#4169E1", lot: 1, index: true },
  { symbol: "CSI300.CN", name: "沪深300", market: "cn", base: 3540, sigma: 0.17, drift: 0.02, brand: "#D2691E", lot: 100, index: true },

  { symbol: "00700.HK", name: "腾讯控股", market: "hk", base: 372, sigma: 0.32, drift: 0.06, brand: "#1E90FF", lot: 1 },
  { symbol: "09988.HK", name: "阿里巴巴", market: "hk", base: 74.2, sigma: 0.40, drift: 0.04, brand: "#FF8C00", lot: 1 },
  { symbol: "03690.HK", name: "美团", market: "hk", base: 112.6, sigma: 0.45, drift: 0.03, brand: "#FFD700", lot: 1 },
  { symbol: "00941.HK", name: "中国移动", market: "hk", base: 71.3, sigma: 0.20, drift: 0.02, brand: "#00BFFF", lot: 1 },
  { symbol: "01810.HK", name: "小米集团", market: "hk", base: 17.8, sigma: 0.48, drift: 0.08, brand: "#FF6347", lot: 1 },
  { symbol: "00388.HK", name: "香港交易所", market: "hk", base: 246, sigma: 0.28, drift: 0.03, brand: "#4682B4", lot: 1 },
  { symbol: "02318.HK", name: "中国平安", market: "hk", base: 36.4, sigma: 0.30, drift: 0.02, brand: "#DAA520", lot: 1 },
  { symbol: "00939.HK", name: "建设银行", market: "hk", base: 5.9, sigma: 0.18, drift: 0.01, brand: "#B0C4DE", lot: 1 },
  { symbol: "01299.HK", name: "友邦保险", market: "hk", base: 54.2, sigma: 0.26, drift: 0.02, brand: "#DC143C", lot: 1 },
  { symbol: "00005.HK", name: "汇丰控股", market: "hk", base: 68.9, sigma: 0.22, drift: 0.01, brand: "#C0C0C0", lot: 1 },
  { symbol: "02015.HK", name: "理想汽车", market: "hk", base: 82.4, sigma: 0.55, drift: 0.05, brand: "#32CD32", lot: 1 },
  { symbol: "09868.HK", name: "小鹏汽车", market: "hk", base: 44.1, sigma: 0.58, drift: 0.03, brand: "#20B2AA", lot: 1 },
  { symbol: "09618.HK", name: "京东集团", market: "hk", base: 104.5, sigma: 0.42, drift: 0.03, brand: "#CD5C5C", lot: 1 },
  { symbol: "09999.HK", name: "网易", market: "hk", base: 142.8, sigma: 0.38, drift: 0.04, brand: "#7B68EE", lot: 1 },
  { symbol: "01347.HK", name: "华虹半导体", market: "hk", base: 15.2, sigma: 0.52, drift: 0.02, brand: "#66CDAA", lot: 1 },

  { symbol: "AAPL.US", name: "Apple", market: "us", base: 214.3, sigma: 0.26, drift: 0.10, brand: "#A9A9A9", lot: 1 },
  { symbol: "NVDA.US", name: "NVIDIA", market: "us", base: 126.4, sigma: 0.50, drift: 0.22, brand: "#76B900", lot: 1 },
  { symbol: "MSFT.US", name: "Microsoft", market: "us", base: 448.1, sigma: 0.24, drift: 0.12, brand: "#00A4EF", lot: 1 },
  { symbol: "TSLA.US", name: "Tesla", market: "us", base: 246.8, sigma: 0.58, drift: 0.06, brand: "#CC0000", lot: 1 },
  { symbol: "AMZN.US", name: "Amazon", market: "us", base: 186.2, sigma: 0.32, drift: 0.11, brand: "#FF9900", lot: 1 },
  { symbol: "GOOGL.US", name: "Alphabet", market: "us", base: 176.5, sigma: 0.28, drift: 0.10, brand: "#4285F4", lot: 1 },
  { symbol: "META.US", name: "Meta", market: "us", base: 504.2, sigma: 0.38, drift: 0.14, brand: "#0668E1", lot: 1 },
  { symbol: "AMD.US", name: "AMD", market: "us", base: 158.7, sigma: 0.48, drift: 0.09, brand: "#ED1C24", lot: 1 },
  { symbol: "NFLX.US", name: "Netflix", market: "us", base: 612.4, sigma: 0.36, drift: 0.10, brand: "#E50914", lot: 1 },
  { symbol: "AVGO.US", name: "Broadcom", market: "us", base: 168.9, sigma: 0.40, drift: 0.15, brand: "#CC092F", lot: 1 },
  { symbol: "JPM.US", name: "JPMorgan", market: "us", base: 204.6, sigma: 0.22, drift: 0.07, brand: "#0F4C81", lot: 1 },
  { symbol: "COIN.US", name: "Coinbase", market: "us", base: 224.8, sigma: 0.65, drift: 0.08, brand: "#0052FF", lot: 1 },
  { symbol: "PLTR.US", name: "Palantir", market: "us", base: 38.4, sigma: 0.60, drift: 0.12, brand: "#101113", lot: 1 },
  { symbol: "BABA.US", name: "阿里巴巴", market: "us", base: 76.9, sigma: 0.40, drift: 0.04, brand: "#FF6A00", lot: 1 },
  { symbol: "NIO.US", name: "蔚来", market: "us", base: 5.1, sigma: 0.62, drift: 0.02, brand: "#00BEFF", lot: 1 },
  { symbol: "BILI.US", name: "哔哩哔哩", market: "us", base: 16.8, sigma: 0.55, drift: 0.03, brand: "#FB7299", lot: 1 },
  { symbol: "PDD.US", name: "拼多多", market: "us", base: 126.3, sigma: 0.46, drift: 0.08, brand: "#E22E1F", lot: 1 },

  { symbol: "600519.CN", name: "贵州茅台", market: "cn", base: 1462, sigma: 0.24, drift: 0.04, brand: "#C71585", lot: 100 },
  { symbol: "300750.CN", name: "宁德时代", market: "cn", base: 182.6, sigma: 0.42, drift: 0.06, brand: "#0F6E3D", lot: 100 },
  { symbol: "601318.CN", name: "中国平安", market: "cn", base: 51.2, sigma: 0.26, drift: 0.02, brand: "#DAA520", lot: 100 },
  { symbol: "000858.CN", name: "五粮液", market: "cn", base: 128.4, sigma: 0.28, drift: 0.02, brand: "#CD853F", lot: 100 },
  { symbol: "002594.CN", name: "比亚迪", market: "cn", base: 236.8, sigma: 0.44, drift: 0.07, brand: "#008B8B", lot: 100 },
  { symbol: "601899.CN", name: "紫金矿业", market: "cn", base: 17.9, sigma: 0.38, drift: 0.09, brand: "#B8860B", lot: 100 },
  { symbol: "600036.CN", name: "招商银行", market: "cn", base: 33.6, sigma: 0.22, drift: 0.03, brand: "#B22222", lot: 100 },
  { symbol: "688981.CN", name: "中芯国际", market: "cn", base: 48.7, sigma: 0.46, drift: 0.05, brand: "#483D8B", lot: 100 },
  { symbol: "002415.CN", name: "海康威视", market: "cn", base: 29.4, sigma: 0.32, drift: 0.02, brand: "#4682B4", lot: 100 },
  { symbol: "300059.CN", name: "东方财富", market: "cn", base: 14.6, sigma: 0.40, drift: 0.04, brand: "#FF8C00", lot: 100 },
];

export const MARKET_LABEL: Record<string, string> = { hk: "港股", us: "美股", cn: "A股" };

/** Five price levels around `last` for the depth ladder. */
export function depthFor(symbol: string, last: number, step: number): number[] {
  const out: number[] = [];
  for (let i = 1; i <= 5; i++) {
    out.push(round2(last - step * i), round2(last + step * i));
  }
  void symbol;
  return out;
}

export function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

const TRADING_DAYS_PER_YEAR = 252;
const DAY_MS = 24 * 3600 * 1000;

/** Seeded daily history. The same symbol always yields the same series. */
export function dailyHistory(ins: Instrument, count: number): Bar[] {
  const rand = mulberry32(hashSeed(ins.symbol));
  const st: GaussState = { spare: null };
  const bars: Bar[] = [];
  let price = ins.base;
  const dtYear = 1 / TRADING_DAYS_PER_YEAR;
  for (let i = count - 1; i >= 0; i--) {
    const open = gapOpen(price, ins, rand, st);
    let high = open;
    let low = open;
    let close = open;
    for (let s = 0; s < 6; s++) {
      close *= stepFactor(ins, dtYear / 6, rand, st);
      high = Math.max(high, close);
      low = Math.min(low, close);
    }
    const volume = Math.floor((0.6 + rand() * 0.9) * (ins.index ? 90_000_000 : 4_000_000));
    bars.push({ t: -(i + 1) * DAY_MS, o: open, h: high, l: low, c: close, v: volume });
    price = close;
  }
  return bars;
}

function hashSeed(symbol: string): number {
  let h = 5381;
  for (let i = 0; i < symbol.length; i++) {
    h = (Math.imul(h, 33) ^ symbol.charCodeAt(i)) >>> 0;
  }
  return h ^ 0x9e3779b9;
}

function stepFactor(ins: Instrument, dtYear: number, rand: () => number, st: GaussState): number {
  const z = gauss(rand, st);
  return Math.exp(
    (ins.drift - (ins.sigma * ins.sigma) / 2) * dtYear + ins.sigma * Math.sqrt(dtYear) * z,
  );
}

function gapOpen(prevClose: number, ins: Instrument, rand: () => number, st: GaussState): number {
  const gap = prevClose * ins.sigma * 0.02 * gauss(rand, st);
  return clampLimits(prevClose * (1 + gap), prevClose, ins);
}

export function clampLimits(price: number, ref: number, ins: Instrument): number {
  if (ins.market === "cn") {
    return Math.min(ref * 1.1, Math.max(ref * 0.9, price));
  }
  const span = ref * ins.sigma * 0.05;
  return Math.min(ref + span, Math.max(ref - span, price));
}

/** Reduces a tick ring into `cols` evenly spaced buckets (last value wins). */
export function downsampleTicks(
  ring: { length: number; at(i: number): { t: number; price: number; volume: number } },
  cols: number,
): number[] {
  const len = ring.length;
  if (len === 0) {
    return [];
  }
  const out = new Array<number>(cols);
  for (let i = 0; i < cols; i++) {
    // Anchor the newest bucket to the newest tick so the sparkline grows leftward.
    const from = Math.floor((i * len) / cols);
    let to = Math.floor(((i + 1) * len) / cols);
    if (to <= from) {
      to = from + 1;
    }
    out[i] = ring.at(Math.min(to, len) - 1).price;
  }
  return out;
}
