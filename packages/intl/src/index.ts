import { setIntlBackend } from "@impasto/i18n";
import { intlDriver as native } from "./driver";
import type { CollatorOptions, DateTimeFormatOptions, NumberFormatOptions, ParsedLocale, PluralCategory, RelativeTimeOptions } from "./types";
export type * from "./types";

export const parseLocale = (tag: string): ParsedLocale => native.parseLocale(tag);
export const matchLocale = (requested: readonly string[], available: readonly string[], fallback: string) =>
  native.matchLocale([...requested], [...available], fallback);
export const pluralCategory = (locale: string, count: number, type?: "cardinal" | "ordinal"): PluralCategory =>
  native.pluralCategory(locale, count, type);
export const formatNumber = (locale: string, value: number, options?: NumberFormatOptions) => native.formatNumber(locale, value, options);
export const formatDateTime = (locale: string, input: number | Date, options?: DateTimeFormatOptions) =>
  native.formatDateTime(locale, Math.trunc(input instanceof Date ? input.getTime() : input), options);
export const formatRelativeTime = (locale: string, value: number, options: RelativeTimeOptions) =>
  native.formatRelativeTime(locale, value, options);
export function compareStrings(locale: string, a: string, b: string, options?: CollatorOptions): -1 | 0 | 1 {
  const order = native.compareStrings(locale, a, b, options);
  return order < 0 ? -1 : order > 0 ? 1 : 0;
}
export const sortStrings = (locale: string, items: readonly string[], options?: CollatorOptions) =>
  native.sortStrings(locale, [...items], options);
export function enableI18nIntl(): void {
  setIntlBackend({ pluralCategory, formatNumber,
    formatCurrency: (locale, value, currency) => formatNumber(locale, value, { style: "currency", currency }),
    formatDate: (locale, value) => formatDateTime(locale, value, { dateStyle: "medium", timeStyle: "short" }),
    formatRelativeTime: (locale, value, unit) => formatRelativeTime(locale, value, { unit, numeric: "auto" }),
  });
}
