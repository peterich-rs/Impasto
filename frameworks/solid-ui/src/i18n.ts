export type Messages = {
  switchOn: string;
  switchOff: string;
};

const ZH: Messages = { switchOn: "开", switchOff: "关" };
const EN: Messages = { switchOn: "On", switchOff: "Off" };

export function messagesFor(locale: string): Messages {
  return locale.startsWith("zh") ? ZH : EN;
}

/** `ar`, `he`, `fa`, and `ur` prefixes, including region suffixes. */
export function directionFor(locale: string): "ltr" | "rtl" {
  const language = locale.toLowerCase().split("-")[0] ?? "";
  if (language === "ar" || language === "he" || language === "fa" || language === "ur") {
    return "rtl";
  }
  return "ltr";
}

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
