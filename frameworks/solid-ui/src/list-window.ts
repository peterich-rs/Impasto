/** Pull past this distance, in scene points, commits refresh on drag end. */
export const REFRESH_THRESHOLD = 64;

export const MIN_THUMB = 24;

/** Finite and positive, otherwise 1. Unknown rows use this instead of a bad estimate. */
export function estimateHeight(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}

/** `thumb = max(24, viewport² / content)`. */
export function thumbLength(viewport: number, content: number): number {
  if (!(content > 0) || !(viewport > 0)) {
    return MIN_THUMB;
  }
  return Math.max(MIN_THUMB, (viewport * viewport) / content);
}

/** `travel * offset / max(content - viewport, 1)`, travel is `viewport - thumb`. */
export function thumbOffset(viewport: number, content: number, offset: number): number {
  const thumb = thumbLength(viewport, content);
  const travel = Math.max(0, viewport - thumb);
  return (travel * offset) / Math.max(content - viewport, 1);
}

/** No next header stays at 0. Otherwise `min(0, nextStart - offset - headerHeight)`. */
export function stickyTranslate(
  nextStart: number | null,
  offset: number,
  headerHeight: number,
): number {
  if (nextStart === null) {
    return 0;
  }
  return Math.min(0, nextStart - offset - headerHeight);
}

/** Only the top edge adds `nextY - rawY`. Anywhere else, including the bottom clamp, adds 0. */
export function pullDelta(liveY: number, rawY: number, nextY: number): number {
  if (liveY <= 0 && rawY < 0) {
    return nextY - rawY;
  }
  return 0;
}

/** Window tail is within `overscan` of `length`. */
export function nearEnd(end: number, length: number, overscan: number): boolean {
  return length - end < overscan;
}

/** Drop height cache entries whose keys are no longer in the list. */
export function retainHeights(cache: Map<string, number>, keys: readonly string[]): void {
  const live = new Set(keys);
  for (const key of cache.keys()) {
    if (!live.has(key)) {
      cache.delete(key);
    }
  }
}

/** Fenwick prefix of row heights. Index 0 is unused. `sum(k)` is the first `k` heights. */

export function highestPowerOfTwo(n: number): number {
  if (n <= 0) {
    return 0;
  }
  let bit = 1;
  while (bit * 2 <= n) {
    bit *= 2;
  }
  return bit;
}

export class PrefixHeights {
  private readonly values: number[] = [];
  private readonly tree: number[] = [0];

  get length(): number {
    return this.values.length;
  }

  heightAt(index: number): number {
    return this.values[index] ?? 0;
  }

  rebuild(heights: readonly number[]): void {
    this.values.length = 0;
    for (const height of heights) {
      this.values.push(height);
    }
    this.reindex();
  }

  private reindex(): void {
    this.tree.length = 0;
    this.tree.push(0);
    for (let index = 0; index < this.values.length; index += 1) {
      this.tree.push(0);
    }
    for (let index = 0; index < this.values.length; index += 1) {
      this.addDelta(index, this.values[index] ?? 0);
    }
  }

  /** Point-update the height at `index` by `delta`. */
  add(index: number, delta: number): void {
    if (index < 0 || index >= this.values.length || delta === 0) {
      return;
    }
    const current = this.values[index] ?? 0;
    this.values[index] = current + delta;
    this.addDelta(index, delta);
  }

  private addDelta(index: number, delta: number): void {
    let cursor = index + 1;
    while (cursor < this.tree.length) {
      this.tree[cursor] = (this.tree[cursor] ?? 0) + delta;
      cursor += cursor & -cursor;
    }
  }

  /** Sum of the first `k` items. `k` counts from 1. */
  sum(k: number): number {
    let cursor = k;
    let total = 0;
    while (cursor > 0) {
      total += this.tree[cursor] ?? 0;
      cursor -= cursor & -cursor;
    }
    return total;
  }

  itemStart(index: number): number {
    return this.sum(index);
  }

  total(): number {
    return this.sum(this.values.length);
  }

  /**
   * Largest `i` in `0..=length` whose prefix is `<= target`.
   * An offset past the total height returns `length`.
   */
  findIndex(target: number): number {
    let index = 0;
    let bit = highestPowerOfTwo(this.values.length);
    let acc = 0;
    while (bit !== 0) {
      const next = index + bit;
      const node = this.tree[next] ?? 0;
      if (next <= this.values.length && acc + node <= target) {
        index = next;
        acc += node;
      }
      bit = Math.floor(bit / 2);
    }
    return index;
  }
}

export type WindowRange = {
  start: number;
  end: number;
  top: number;
  bottom: number;
};

/** Visible slice plus overscan. `end` is exclusive. */
export function windowRange(
  prefix: PrefixHeights,
  offset: number,
  viewHeight: number,
  overscan: number,
): WindowRange {
  const count = prefix.length;
  if (count <= 0) {
    return { start: 0, end: 0, top: 0, bottom: 0 };
  }
  let start = prefix.findIndex(offset);
  if (start === count) {
    return { start: count, end: count, top: prefix.total(), bottom: 0 };
  }
  let end = start;
  const limit = offset + Math.max(viewHeight, 0);
  while (end < count && prefix.itemStart(end) < limit) {
    end += 1;
  }
  start = Math.max(0, start - overscan);
  end = Math.min(count, end + overscan);
  return {
    start,
    end,
    top: prefix.itemStart(start),
    bottom: prefix.total() - prefix.itemStart(end),
  };
}
