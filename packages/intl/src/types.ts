export type PluralCategory = "zero" | "one" | "two" | "few" | "many" | "other";
export type TextDirection = "ltr" | "rtl";
export type FormatStyle = "short" | "medium" | "long" | "full";
export type IntlErrorCode = "invalid-locale" | "invalid-timezone" | "invalid-currency" | "invalid-input" | "missing-locale-data" | "host-unavailable";

export interface ParsedLocale {
  readonly canonical: string;
  readonly language: string;
  readonly script: string | null;
  readonly region: string | null;
  readonly direction: TextDirection;
}

export interface NumberFormatOptions {
  readonly style?: "decimal" | "currency" | "percent";
  readonly currency?: string; // ISO 4217，如 "CNY" / "USD" / "JPY"
  readonly currencyDisplay?: "symbol" | "narrowSymbol" | "code" | "name";
  readonly notation?: "standard" | "compactShort" | "compactLong";
  readonly minimumFractionDigits?: number;
  readonly maximumFractionDigits?: number;
  readonly useGrouping?: boolean; // 默认 true
}

export interface DateTimeFormatOptions {
  readonly dateStyle?: FormatStyle;
  readonly timeStyle?: FormatStyle;
  readonly timeZone?: string; // IANA 时区名，如 "Asia/Shanghai" / "UTC"；缺省取系统当前时区
  readonly hour12?: boolean;
}

export type RelativeTimeUnit = "second" | "minute" | "hour" | "day" | "week" | "month" | "quarter" | "year";
export interface RelativeTimeOptions {
  readonly unit: RelativeTimeUnit;
  readonly style?: "long" | "short" | "narrow";
  readonly numeric?: "always" | "auto"; // auto 支持输出 "昨天" / "yesterday"
}

export interface CollatorOptions {
  readonly sensitivity?: "base" | "accent" | "case" | "variant";
  readonly numeric?: boolean; // "item 2" < "item 10"
}
