import { host } from "@impasto/solid-renderer";
import type { QuoteEngine } from "../engine/engine.ts";
import type { Quote } from "../engine/types.ts";
import { toneFor, type Updown } from "./settings.ts";
import { fmtPct, fmtPrice } from "../format.ts";
import { flash } from "./flash.ts";

type RGBA = [number, number, number, number];

/** Cell node ids one mounted row hands to the binder. */
export type QuoteCells = {
  last?: number;
  pct?: number;
  chg?: number;
  flashNode?: number;
};

export type SparkRefresh = () => void;

type CellBinding = {
  symbol: string;
  cells: QuoteCells;
  revision: number;
};

type SparkBinding = {
  symbol: string;
  refresh: SparkRefresh;
};

/**
 * Registry of mounted quote cells. The hot path never touches Solid: each
 * vsync, for every mounted row whose `revision` moved, write text and colors
 * straight through `host`. One symbol may have several bindings (index strip
 * and a list row). Unbind is by entry identity, so last and pct do not
 * overwrite each other. Rows that scrolled out are simply absent.
 */
export class QuoteBinder {
  readonly #engine: QuoteEngine;
  readonly #bindings: CellBinding[] = [];
  readonly #sparks: SparkBinding[] = [];
  #up: RGBA;
  #down: RGBA;
  #flat: RGBA;
  #base: RGBA;
  #updown: () => Updown;

  constructor(
    engine: QuoteEngine,
    colors: { up: RGBA; down: RGBA; flat: RGBA; base: RGBA },
    updown: () => Updown,
  ) {
    this.#engine = engine;
    this.#up = colors.up;
    this.#down = colors.down;
    this.#flat = colors.flat;
    this.#base = colors.base;
    this.#updown = updown;
  }

  setColors(colors: { up: RGBA; down: RGBA; flat: RGBA; base: RGBA }): void {
    this.#up = colors.up;
    this.#down = colors.down;
    this.#flat = colors.flat;
    this.#base = colors.base;
    this.rewrite();
  }

  /** Re-writes every mounted row with current values. Used after palette or convention flips. */
  rewrite(): void {
    for (const entry of this.#bindings) {
      entry.revision = 0;
      this.#sync(entry);
    }
    for (const spark of this.#sparks) {
      this.#runSpark(spark);
    }
  }

  bind(symbol: string, cells: QuoteCells): () => void {
    const entry: CellBinding = { symbol, cells, revision: 0 };
    this.#bindings.push(entry);
    this.#sync(entry);
    return () => this.#dropCells(entry);
  }

  bindSpark(symbol: string, refresh: SparkRefresh): () => void {
    const entry: SparkBinding = { symbol, refresh };
    this.#sparks.push(entry);
    this.#runSpark(entry);
    return () => {
      const i = this.#sparks.indexOf(entry);
      if (i >= 0) {
        this.#sparks.splice(i, 1);
      }
    };
  }

  /** Called once per vsync after `engine.step`. */
  flush(): void {
    for (const entry of [...this.#bindings]) {
      if (entry.revision === this.#engine.quote(entry.symbol).revision) {
        continue;
      }
      this.#sync(entry);
    }
    flash.tick(this.#base);
  }

  #dropCells(entry: CellBinding): void {
    const i = this.#bindings.indexOf(entry);
    if (i >= 0) {
      this.#bindings.splice(i, 1);
    }
    const c = entry.cells;
    if (c.last !== undefined) {
      flash.release(c.last);
    }
    if (c.pct !== undefined) {
      flash.release(c.pct);
    }
    if (c.chg !== undefined) {
      flash.release(c.chg);
    }
    if (c.flashNode !== undefined) {
      flash.release(c.flashNode);
    }
  }

  /** A dead node must not throw: that removes the market `onFrame` and halts Solid. */
  #sync(entry: CellBinding): void {
    const q = this.#engine.quote(entry.symbol);
    try {
      this.#writeOne(entry.cells, q);
      entry.revision = q.revision;
    } catch {
      this.#dropCells(entry);
    }
  }

  #runSpark(entry: SparkBinding): void {
    try {
      entry.refresh();
    } catch {
      const i = this.#sparks.indexOf(entry);
      if (i >= 0) {
        this.#sparks.splice(i, 1);
      }
    }
  }

  #writeOne(c: QuoteCells, q: Quote): void {
    const tone = toneFor(q.last, q.prevClose, this.#updown());
    const color = tone === "up" ? this.#up : tone === "down" ? this.#down : this.#flat;
    if (c.last !== undefined) {
      host.setText(c.last, fmtPrice(q.last));
    }
    if (c.pct !== undefined) {
      host.setText(c.pct, fmtPct(q.last, q.prevClose));
    }
    if (c.chg !== undefined) {
      host.setText(c.chg, fmtPrice(q.last - q.prevClose));
    }
    if (c.flashNode !== undefined) {
      if (c.last !== undefined) {
        host.setColor(c.flashNode, color);
      }
      if (tone !== "flat") {
        flash.restart(c.flashNode, color);
      }
    }
  }
}
