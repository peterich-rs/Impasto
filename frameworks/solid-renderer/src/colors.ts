interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

function parseColor(value: unknown): Rgba {
  if (typeof value === "string") {
    const hex = value.startsWith("#") ? value.slice(1) : value;
    if (hex.length !== 6 && hex.length !== 8) {
      throw new Error(`bad color ${value}`);
    }
    const r = Number.parseInt(hex.slice(0, 2), 16);
    const g = Number.parseInt(hex.slice(2, 4), 16);
    const b = Number.parseInt(hex.slice(4, 6), 16);
    const a = hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) : 255;
    return { r, g, b, a };
  }
  if (value !== null && typeof value === "object" && "r" in value && "g" in value && "b" in value) {
    const color = value as { r: number; g: number; b: number; a?: number };
    return { r: color.r, g: color.g, b: color.b, a: color.a ?? 255 };
  }
  throw new Error("color must be #RRGGBB, #RRGGBBAA, or {r,g,b,a}");
}

export function colorBytes(value: unknown): [number, number, number, number] {
  const color = parseColor(value);
  return [color.r, color.g, color.b, color.a];
}
