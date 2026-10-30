/** Hot-path number formatting. Pre-built tables keep allocation off the vsync path. */

const GROUPS = [8, 16, 24, 32];

function integerDigits(value: number): string {
  const s = String(Math.trunc(Math.abs(value)));
  return s;
}

/** Thousands-grouped integer part. zh keeps grouping too (exchange convention). */
function grouped(value: number): string {
  const digits = integerDigits(value);
  if (digits.length <= 3) {
    return digits;
  }
  let out = "";
  const head = digits.length % 3;
  if (head > 0) {
    out = digits.slice(0, head);
    for (let i = head; i < digits.length; i += 3) {
      out += "," + digits.slice(i, i + 3);
    }
    return out;
  }
  for (let i = 0; i < digits.length; i += 3) {
    out += (i === 0 ? "" : ",") + digits.slice(i, i + 3);
  }
  return out;
}

/** Two fractional digits, no grouping on the fraction. */
export function fmtPrice(value: number): string {
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const yuan = Math.floor(abs);
  const cents = Math.round((abs - yuan) * 100);
  if (cents === 100) {
    return `${sign}${grouped(yuan + 1)}.00`;
  }
  return `${sign}${grouped(yuan)}.${cents < 10 ? "0" + cents : String(cents)}`;
}

/** Signed percent with two digits. */
export function fmtPct(last: number, prevClose: number): string {
  const pct = prevClose === 0 ? 0 : ((last - prevClose) / prevClose) * 100;
  const sign = pct > 0.0001 ? "+" : "";
  return `${sign}${pct.toFixed(2)}%`;
}

export function fmtSigned(value: number): string {
  const sign = value > 0.0001 ? "+" : "";
  return sign + fmtPrice(value);
}

/** Compact volume: 1.23亿 / 4567万 / 8901. */
export function fmtVolumeCN(volume: number): string {
  if (volume >= 100_000_000) {
    return `${(volume / 100_000_000).toFixed(2)}亿`;
  }
  if (volume >= 10_000) {
    return `${(volume / 10_000).toFixed(2)}万`;
  }
  return String(Math.round(volume));
}

export function fmtQty(qty: number): string {
  return grouped(qty);
}

void GROUPS;
