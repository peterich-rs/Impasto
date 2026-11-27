import type { ViewStyle } from "@impasto/solid-renderer";
import { loc } from "../../copy.ts";
import { settingsSig } from "../../state/settings.ts";
import type { Sector } from "../../engine/analytics.ts";
import type { Quote } from "../../engine/types.ts";

export const colors = {
  bg: "#171A20", bar: "#1E2228", raised: "#32353B", line: "#30343B",
  ink: "#EEF0F4", dim: "#9DA2AA", faint: "#80858E",
  lime: "#C6EF71", up: "#43D3A0", down: "#FF6276", amber: "#FFB548", blue: "#7EAAFF",
};

export const filling: ViewStyle = { flex: 1, minHeight: 0, minWidth: 0, gap: 0 };

export function change(quote: Pick<Quote, "last" | "prevClose">): number {
  return (quote.last - quote.prevClose) / quote.prevClose * 100;
}

export function sign(value: number, digits = 2): string {
  return `${value >= 0 ? "+" : ""}${value.toFixed(digits)}`;
}

export function tone(value: number): string {
  const reversed = settingsSig().updown === "cn";
  return value === 0 ? colors.dim : (value > 0) !== reversed ? colors.up : colors.down;
}

export function compact(value: number): string {
  const magnitude = Math.abs(value);
  return magnitude >= 1_000_000_000 ? `${(value / 1_000_000_000).toFixed(2)}B`
    : magnitude >= 1_000_000 ? `${(value / 1_000_000).toFixed(2)}M`
    : magnitude >= 10_000 ? `${(value / 1_000).toFixed(1)}K` : String(Math.round(value));
}

export function clock(ms: number, seconds = true): string {
  const sec = Math.max(0, Math.floor(ms / 1000));
  const part = (n: number) => String(n).padStart(2, "0");
  return `${part(Math.floor(sec / 3600) % 24)}:${part(Math.floor(sec / 60) % 60)}${seconds ? `:${part(sec % 60)}` : ""}`;
}

export function code(symbol: string): string {
  return symbol.split(".")[0]!;
}

export function sectorLabel(sector: Sector | undefined): string {
  switch (sector) {
    case "technology": return loc("科技", "Technology");
    case "health": return loc("医疗", "Health");
    case "finance": return loc("金融", "Finance");
    case "consumer": return loc("消费", "Consumer");
    case "industrials": return loc("工业", "Industrials");
    case "energy": return loc("能源", "Energy");
    default: return loc("其他", "Other");
  }
}
