import { directionFor } from "./i18n.ts";

/** Map logical start/end padding to physical left/right. */
export function resolvePadding(
  direction: "ltr" | "rtl",
  start: number,
  end: number,
): { left: number; right: number } {
  if (direction === "rtl") {
    return { left: end, right: start };
  }
  return { left: start, right: end };
}

export function resolvePaddingForLocale(
  locale: string,
  start: number,
  end: number,
): { left: number; right: number } {
  return resolvePadding(directionFor(locale), start, end);
}
