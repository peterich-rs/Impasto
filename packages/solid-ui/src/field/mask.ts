export type InputMask = {
  /** Format a committed string. Not applied while a composition is open. */
  format(value: string): string;
};

function digitsOnly(value: string): string {
  return value.replace(/\D/g, "");
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

/** Digits and a single decimal point. */
export function decimal(): InputMask {
  return {
    format(value: string) {
      const negative = value.trim().startsWith("-");
      const cleaned = value.replace(/[^\d.]/g, "");
      const dot = cleaned.indexOf(".");
      const whole = (dot < 0 ? cleaned : cleaned.slice(0, dot)).replace(/^0+(?=\d)/, "");
      const fraction = dot < 0 ? "" : cleaned.slice(dot + 1).replace(/\./g, "");
      const body = dot < 0 ? whole : `${whole || "0"}.${fraction}`;
      if (body.length === 0) {
        return negative ? "-" : "";
      }
      return `${negative ? "-" : ""}${body}`;
    },
  };
}

/** Ten-digit groups, `(555) 123-4567`. Shorter input keeps the partial grouping. */
export function phone(): InputMask {
  return {
    format(value: string) {
      const digits = digitsOnly(value).slice(0, 10);
      if (digits.length === 0) {
        return "";
      }
      if (digits.length < 4) {
        return `(${digits}`;
      }
      if (digits.length < 7) {
        return `(${digits.slice(0, 3)}) ${digits.slice(3)}`;
      }
      return `(${digits.slice(0, 3)}) ${digits.slice(3, 6)}-${digits.slice(6)}`;
    },
  };
}

/** `$1,234.56`. The sign is preserved. */
export function currency(): InputMask {
  return {
    format(value: string) {
      const negative = value.trim().startsWith("-");
      const cleaned = value.replace(/[^\d.]/g, "");
      const dot = cleaned.indexOf(".");
      const wholeDigits = (dot < 0 ? cleaned : cleaned.slice(0, dot)) || "0";
      const fraction = (dot < 0 ? "" : cleaned.slice(dot + 1).replace(/\./g, "")) + "00";
      const whole = groupThousands(wholeDigits.replace(/^0+(?=\d)/, "") || "0");
      return `${negative ? "-" : ""}$${whole}.${fraction.slice(0, 2)}`;
    },
  };
}
