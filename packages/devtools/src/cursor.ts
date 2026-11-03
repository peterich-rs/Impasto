/** Append `fresh` and keep at most `cap` entries. Empty fresh is a no-op. */
export function appendCapped<T>(current: readonly T[], fresh: readonly T[], cap: number): T[] {
  if (fresh.length === 0) {
    return current as T[];
  }
  const merged = current.length === 0 ? (fresh as T[]) : [...current, ...fresh];
  return merged.length > cap ? merged.slice(merged.length - cap) : merged;
}
