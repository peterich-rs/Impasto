/** Deterministic random sources. Every draw in the engine comes from a
 * per-symbol seeded stream so tests can replay any sequence. */

export function hashString(value: string): number {
  let h = 2166136261;
  for (let i = 0; i < value.length; i++) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export type GaussState = { spare: number | null };

/** Marsaglia polar method. Uses one draw for two samples. */
export function gauss(rand: () => number, state: GaussState): number {
  if (state.spare !== null) {
    const value = state.spare;
    state.spare = null;
    return value;
  }
  let u = 0;
  let v = 0;
  let s = 0;
  do {
    u = rand() * 2 - 1;
    v = rand() * 2 - 1;
    s = u * u + v * v;
  } while (s >= 1 || s === 0);
  const factor = Math.sqrt((-2 * Math.log(s)) / s);
  state.spare = v * factor;
  return u * factor;
}
