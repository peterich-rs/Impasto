function clampDigits(fractionDigits: number): number {
  if (!Number.isFinite(fractionDigits)) {
    return 0;
  }
  return Math.min(6, Math.max(0, Math.trunc(fractionDigits)));
}

/** Half away from zero, then group. `zh` has no thousands separator. */
export function formatNumber(locale: string, value: number, fractionDigits: number): string {
  if (!Number.isFinite(value)) {
    return "";
  }
  const digits = clampDigits(fractionDigits);
  const sign = value < 0 ? "-" : "";
  const factor = 10 ** digits;
  const rounded = Math.round(Math.abs(value) * factor) / factor;
  const [whole, fraction = ""] = rounded.toFixed(digits).split(".");
  const grouped = locale.startsWith("zh") ? whole : groupThousands(whole ?? "0");
  if (digits === 0) {
    return `${sign}${grouped}`;
  }
  return `${sign}${grouped}.${fraction}`;
}

function groupThousands(whole: string): string {
  const digits = whole.replace("-", "");
  let out = "";
  for (let index = 0; index < digits.length; index += 1) {
    if (index > 0 && (digits.length - index) % 3 === 0) {
      out += ",";
    }
    out += digits[index];
  }
  return out;
}

export function plural(
  locale: string,
  n: number,
  forms: { one: string; other: string },
): string {
  if (locale.startsWith("zh")) {
    return forms.other;
  }
  return n === 1 ? forms.one : forms.other;
}

function pad(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

/** `YYYY-MM-DD HH:mm` in the runtime's local zone. */
export function formatDate(_locale: string, epochMs: number): string {
  const date = new Date(epochMs);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

import { scene } from "scene";
import { createSignal } from "solid-js";
import { defaultLocale, messages } from "virtual:impasto-locales";

declare global {
  interface ImpastoMessages {}
}

export type MessageKey = keyof ImpastoMessages extends never ? string : keyof ImpastoMessages;
type MessageParams = Record<string, string | number>;
type ArgumentsFor<K> = K extends keyof ImpastoMessages
  ? keyof ImpastoMessages[K] extends never ? [] : [params: ImpastoMessages[K]]
  : [params?: MessageParams];
type Forms = string | { one: string; other: string; zero?: string };

const [locale, setLocale] = createSignal<string | undefined>();
const [systemLocale, setSystemLocale] = createSignal<string | undefined>();

/** Override the system locale. Pass undefined to follow system settings. */
export function setLanguage(language: string | undefined): void {
  setLocale(language?.toLowerCase());
}

function activeLocale(): string {
  const selected = locale();
  if (selected) return selected;
  if (systemLocale() === undefined) {
    setSystemLocale(scene.settings().locale.toLowerCase());
    scene.onSettings(settings => setSystemLocale(settings.locale.toLowerCase()));
  }
  return systemLocale() ?? defaultLocale;
}

/** Format one compiler-indexed message using the active language. */
export function translateMessage(catalog: Record<string, Forms>, params: MessageParams = {}): string {
  let language = activeLocale();
  while (!Object.hasOwn(catalog, language) && language.includes("-")) {
    language = language.slice(0, language.lastIndexOf("-"));
  }
  if (!Object.hasOwn(catalog, language)) language = defaultLocale;
  const message = catalog[language];
  const count = params.count;
  const text = typeof message === "string"
    ? message
    : message && (count === 0 && message.zero !== undefined
      ? message.zero : plural(language, typeof count === "number" ? count : NaN, message));
  return text?.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (placeholder, name: string) =>
    Object.hasOwn(params, name) ? String(params[name]) : placeholder) ?? "";
}

/** Translate an application message and interpolate its named parameters. */
export function t<K extends MessageKey>(key: K, ...args: ArgumentsFor<K>): string;
export function t(key: string, params?: MessageParams): string {
  return translateCatalog(messages, key, params);
}

/** Resolve a dynamic key in the catalog selected by the compiler. */
export function translateCatalog(catalog: Record<string, Record<string, Forms>>, key: string, params?: MessageParams): string {
  return Object.hasOwn(catalog, key) ? translateMessage(catalog[key], params) : key;
}
