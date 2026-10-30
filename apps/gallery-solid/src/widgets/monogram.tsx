import { host, type NodeHandle } from "@impasto/solid-renderer";
import type { Instrument } from "../engine/types.ts";
import { colorBytes } from "../colors.ts";

const MONO: Record<string, [number, number, number, number]> = {};

function brandOf(ins: Instrument): [number, number, number, number] {
  const cached = MONO[ins.symbol];
  if (cached) {
    return cached;
  }
  const bytes = colorBytes(ins.brand);
  MONO[ins.symbol] = bytes;
  return bytes;
}

function labelOf(ins: Instrument): string {
  if (ins.index) {
    return ins.name.slice(0, 2);
  }
  if (ins.market === "us") {
    const latin = ins.symbol.replace(".US", "");
    return latin.slice(0, 2).toUpperCase();
  }
  if (ins.market === "cn") {
    return ins.name.slice(0, 2);
  }
  return ins.symbol.slice(0, 2);
}

/** Two-letter monogram on the brand color. No bitmap assets. */
export function Monogram(props: { ins: () => Instrument; size?: number; fontSize?: number }): NodeHandle {
  const size = props.size ?? 36;
  const font = props.fontSize ?? 12;
  const ins = props.ins();
  const box = host.decorated(brandOf(ins), size / 3);
  host.setMinSize(box, size, size);
  const label = host.text(labelOf(ins), font, [255, 255, 255, 255]);
  host.append(box, label);
  return { id: box };
}
