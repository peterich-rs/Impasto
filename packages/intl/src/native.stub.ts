import type { CollatorOptions, DateTimeFormatOptions, NumberFormatOptions, ParsedLocale, RelativeTimeOptions } from "./types";

export const isIntlHostAvailable = () => false;
export function parseLocale(tag: string): ParsedLocale {
  const locale = new Intl.Locale(tag);
  const script = locale.maximize().script;
  return { canonical: locale.toString(), language: locale.language, script: locale.script ?? null, region: locale.region ?? null,
    direction: ["Arab", "Hebr", "Thaa", "Nkoo", "Adlm", "Syrc", "Samr"].includes(script ?? "") ? "rtl" : "ltr" };
}
export function matchLocale(requested: string[], available: string[], fallback: string): string {
  const candidates = available.map(tag => new Intl.Locale(tag));
  for (const tag of requested) {
    const target = new Intl.Locale(tag);
    const match = candidates.find(item => item.baseName === target.baseName)
      ?? candidates.find(item => item.language === target.language && item.maximize().script === target.maximize().script)
      ?? candidates.find(item => item.language === target.language);
    if (match) return match.toString();
  }
  return new Intl.Locale(fallback).toString();
}
export const pluralCategory = (locale: string, value: number, type?: "cardinal" | "ordinal") =>
  new Intl.PluralRules(locale, { type }).select(value);
export function formatNumber(locale: string, value: number, options?: NumberFormatOptions): string {
  const { notation, ...rest } = options ?? {};
  return new Intl.NumberFormat(locale, { ...rest, notation: notation && notation !== "standard" ? "compact" : "standard",
    compactDisplay: notation === "compactLong" ? "long" : "short" }).format(value);
}
export const formatDateTime = (locale: string, epoch: number, options?: DateTimeFormatOptions) =>
  new Intl.DateTimeFormat(locale, options).format(epoch);
export function formatRelativeTime(locale: string, value: number, { unit, ...options }: RelativeTimeOptions) {
  return new Intl.RelativeTimeFormat(locale, options).format(value, unit);
}
export const compareStrings = (locale: string, a: string, b: string, options?: CollatorOptions) =>
  new Intl.Collator(locale, options).compare(a, b);
export const sortStrings = (locale: string, values: string[], options?: CollatorOptions) =>
  [...values].sort(new Intl.Collator(locale, options).compare);
