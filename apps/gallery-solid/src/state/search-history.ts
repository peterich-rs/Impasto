import { createSignal } from "solid-js";

const KEY = "impasto.quotes.search.v1";
const CAP = 8;

const [history, setHistory] = createSignal<string[]>([]);
const [loaded, setLoaded] = createSignal(false);

export const searchHistorySig = history;

export const HOT_SYMBOLS = ["00700.HK", "AAPL.US", "600519.CN", "NVDA.US", "TSLA.US", "09988.HK"];

export function pushSearchHistory(term: string): void {
  const next = term.trim();
  if (next.length === 0) {
    return;
  }
  setHistory((list) => [next, ...list.filter((item) => item !== next)].slice(0, CAP));
  void persist();
}

export function clearSearchHistory(): void {
  setHistory([]);
  void persist();
}

async function persist(): Promise<void> {
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    await AsyncStorage.multiSet([[KEY, JSON.stringify(history())]]);
  } catch {
    /* unlinked host */
  }
}

export async function loadSearchHistory(): Promise<void> {
  if (loaded()) {
    return;
  }
  setLoaded(true);
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    const [[, raw]] = await AsyncStorage.multiGet([KEY]);
    if (raw) {
      const parsed = JSON.parse(raw) as unknown;
      if (Array.isArray(parsed) && parsed.every((item) => typeof item === "string")) {
        setHistory((parsed as string[]).slice(0, CAP));
      }
    }
  } catch {
    /* unlinked host */
  }
}
