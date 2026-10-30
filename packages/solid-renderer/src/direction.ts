/** `ar`, `he`, `fa`, and `ur` prefixes, including region suffixes. */
export function directionFor(locale: string): "ltr" | "rtl" {
  const language = locale.toLowerCase().split("-")[0] ?? "";
  if (language === "ar" || language === "he" || language === "fa" || language === "ur") {
    return "rtl";
  }
  return "ltr";
}
