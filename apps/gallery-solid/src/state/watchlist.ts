import { createSignal } from "solid-js";

const KEY = "impasto.quotes.watchlist.v1";

/** Watchlist state. Persisted via async-storage; falls back to memory when unlinked. */
const DEFAULT = ["HSI.HK", "HSTECH.HK", "SPX.US", "NDX.US", "CSI300.CN", "00700.HK", "AAPL.US", "NVDA.US", "600519.CN", "300750.CN", "TSLA.US", "09988.HK"];

const [watchlist, setWatchlist] = createSignal<string[]>(DEFAULT);
const [loaded, setLoaded] = createSignal(false);

export const watchlistSig = watchlist;
export const watchlistLoaded = loaded;

export function inWatchlist(symbol: string): boolean {
  return watchlist().includes(symbol);
}

export function toggleWatch(symbol: string): void {
  setWatchlist((list) =>
    list.includes(symbol) ? list.filter((s) => s !== symbol) : [...list, symbol],
  );
  void persist();
}

async function persist(): Promise<void> {
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    await AsyncStorage.multiSet([[KEY, JSON.stringify(watchlist())]]);
  } catch {
    /* unlinked host */
  }
}

export async function loadWatchlist(): Promise<void> {
  if (loaded()) {
    return;
  }
  setLoaded(true);
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    const [[, raw]] = await AsyncStorage.multiGet([KEY]);
    if (raw) {
      const parsed = JSON.parse(raw) as unknown;
      if (Array.isArray(parsed) && parsed.every((s) => typeof s === "string")) {
        setWatchlist(parsed as string[]);
      }
    }
  } catch {
    /* unlinked host */
  }
}
