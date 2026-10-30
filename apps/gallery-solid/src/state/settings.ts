import { createSignal } from "solid-js";

export type Updown = "cn" | "intl";

export type Settings = {
  updown: Updown;
  locale: "zh" | "en";
  timeScale: number;
};

const KEY = "impasto.quotes.settings.v1";

const [settings, setSettings] = createSignal<Settings>({ updown: "cn", locale: "zh", timeScale: 60 });
const [loaded, setLoaded] = createSignal(false);

export const settingsSig = settings;
export const settingsLoaded = loaded;

export function updateSettings(patch: Partial<Settings>): void {
  setSettings((prev) => ({ ...prev, ...patch }));
  void persist();
}

async function persist(): Promise<void> {
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    await AsyncStorage.multiSet([[KEY, JSON.stringify(settings())]]);
  } catch {
    /* unlinked host: keep the in-memory copy */
  }
}

export async function loadSettings(): Promise<void> {
  if (loaded()) {
    return;
  }
  setLoaded(true);
  try {
    const { AsyncStorage } = await import("@impasto/async-storage");
    const [[, raw]] = await AsyncStorage.multiGet([KEY]);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Settings>;
      setSettings((prev) => ({
        ...prev,
        updown: parsed.updown === "intl" ? "intl" : "cn",
        locale: parsed.locale === "en" ? "en" : "zh",
        timeScale: parsed.timeScale && parsed.timeScale > 0 ? parsed.timeScale : prev.timeScale,
      }));
    }
  } catch {
    /* unlinked host */
  }
}

/** Up/down color convention. cn: red rises. intl: green rises. */
export function toneFor(last: number, prevClose: number, updown: Updown): "up" | "down" | "flat" {
  if (last > prevClose) {
    return updown === "cn" ? "up" : "down";
  }
  if (last < prevClose) {
    return updown === "cn" ? "down" : "up";
  }
  return "flat";
}
