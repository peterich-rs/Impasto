/** Color parsing for the app. Mirrors solid-ui colors. */
export function colorBytes(hex: string): [number, number, number, number] {
  const h = hex.startsWith("#") ? hex.slice(1) : hex;
  const full = h.length === 3
    ? h
      .split("")
      .map((c) => c + c)
      .join("")
    : h;
  return [
    Number.parseInt(full.slice(0, 2), 16),
    Number.parseInt(full.slice(2, 4), 16),
    Number.parseInt(full.slice(4, 6), 16),
    full.length === 8 ? Number.parseInt(full.slice(6, 8), 16) : 255,
  ];
}
