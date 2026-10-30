/** `#RRGGBB` or `#RRGGBBAA` to 0–255 channels. */
export function colorBytes(hex: string): number[] {
  const h = hex.startsWith("#") ? hex.slice(1) : hex;
  if (h.length === 8) {
    return [
      Number.parseInt(h.slice(0, 2), 16),
      Number.parseInt(h.slice(2, 4), 16),
      Number.parseInt(h.slice(4, 6), 16),
      Number.parseInt(h.slice(6, 8), 16),
    ];
  }
  return [
    Number.parseInt(h.slice(0, 2), 16),
    Number.parseInt(h.slice(2, 4), 16),
    Number.parseInt(h.slice(4, 6), 16),
    255,
  ];
}
