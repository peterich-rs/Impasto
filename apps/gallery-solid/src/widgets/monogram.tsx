import { scene, type NodeHandle } from "@impasto/solid-renderer";
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
export function Monogram(props: {
  ins: () => Instrument;
  size?: number;
  fontSize?: number;
}): NodeHandle {
  const size = props.size ?? 36;
  const font = props.fontSize ?? 12;
  const ins = props.ins();
  const box = scene.createElement("view", {
    style: {
      backgroundColor: brandOf(ins),
      borderRadius: size / 3,
    },
  });
  scene.patchStyle(box, {
    minWidth: size,
    minHeight: size,
  });
  const label = scene.createElement("text", {
    value: labelOf(ins),
    style: {
      fontSize: font,
      color: [255, 255, 255, 255],
    },
  });
  const align = scene.createElement("view", {
    style: {
      alignItems: "center",
      justifyContent: "center",
    },
  });
  scene.patchStyle(align, {
    flex: 1,
  });
  scene.append(align, label);
  scene.append(box, align);
  return box;
}
