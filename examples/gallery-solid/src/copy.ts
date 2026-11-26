import { settingsSig } from "./state/settings.ts";

/** App copy. Settings `locale` picks the side; missing keys stay Chinese. */
export function loc(zh: string, en: string): string {
  return settingsSig().locale === "en" ? en : zh;
}
