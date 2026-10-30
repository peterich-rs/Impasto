import { host } from "@impasto/solid-renderer";

type RGBA = [number, number, number, number];

/**
 * Price flash: on each new tick the text jumps to the tone color and decays
 * back to the base text color over 300ms. A newer tick on the same cell
 * restarts the decay. Runs on the frame clock, never `performance.now()`.
 */
class FlashClock {
  readonly #active = new Map<number, { from: RGBA; start: number }>();
  #now = 0;
  #reduced = false;

  /** The provider stamps the frame clock here before flushing. */
  stamp(targetMs: number): void {
    this.#now = targetMs;
  }

  setReduced(reduced: boolean): void {
    this.#reduced = reduced;
    if (reduced) {
      this.#active.clear();
    }
  }

  restart(node: number, tone: RGBA): void {
    if (this.#reduced) {
      return; // static tone color already written by the binder
    }
    this.#active.set(node, { from: tone, start: this.#now });
  }

  /** Drop a cell that has been unbound or discarded. */
  release(node: number): void {
    this.#active.delete(node);
  }

  tick(base: RGBA): void {
    if (this.#active.size === 0) {
      return;
    }
    const DURATION = 300;
    for (const [node, f] of [...this.#active]) {
      const p = Math.max(0, Math.min(1, (this.#now - f.start) / DURATION));
      const eased = 1 - Math.pow(1 - p, 3); // easeOutCubic
      try {
        host.setColor(node, mix(f.from, base, eased));
      } catch {
        this.#active.delete(node);
        continue;
      }
      if (p >= 1) {
        this.#active.delete(node);
      }
    }
  }
}

function mix(a: RGBA, b: RGBA, t: number): RGBA {
  return [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
    255,
  ];
}

export const flash = new FlashClock();
