import { host } from "@impasto/solid-renderer";
import { candleColor, candleRects, visibleRange, type ChartGeom } from "./geometry.ts";

type Slots = {
  wrap: number;
  wick: number;
  body: number;
  vol: number;
  index: number;
};

const POOL_MAX = 260;

/**
 * Horizontal candle window. Bars inside `[first, last]` are mounted as
 * absolutely positioned wraps; out-of-window wraps are detached (kept alive)
 * into a pool and re-`append`ed when the window returns. `update` runs at
 * most once per vsync and only when scroll/scale/revision moved.
 */
export class ChartWindow {
  #parent = 0;
  #mounted = new Map<number, Slots>();
  #pool: Slots[] = [];
  #lastKey = "";

  attach(parent: number): void {
    this.#parent = parent;
  }

  mountedCount(): number {
    return this.#mounted.size;
  }

  update(scrollX: number, viewW: number, geom: ChartGeom, forceKey: string): number {
    const key = forceKey;
    if (key === this.#lastKey && this.#mounted.size > 0) {
      return 0;
    }
    this.#lastKey = key;
    const [first, last] = visibleRange(scrollX, viewW, geom);
    let writes = 0;

    for (const [i, slots] of this.#mounted) {
      if (i < first || i > last) {
        host.detach(this.#parent, slots.wrap);
        this.#park(slots);
        this.#mounted.delete(i);
      }
    }
    for (let i = first; i <= last; i++) {
      let slots = this.#mounted.get(i);
      if (slots === undefined) {
        slots = this.#pool.pop() ?? this.#create();
        host.append(this.#parent, slots.wrap);
        this.#mounted.set(i, slots);
      }
      this.#paint(i, slots, geom);
      writes += 1;
    }
    return writes;
  }

  /** Re-paints every mounted candle after a palette or convention flip. */
  repaint(geom: ChartGeom): void {
    for (const [i, slots] of this.#mounted) {
      this.#paint(i, slots, geom);
    }
  }

  release(): void {
    for (const slots of this.#mounted.values()) {
      host.discard(slots.wrap);
    }
    this.#mounted.clear();
    for (const slots of this.#pool) {
      host.discard(slots.wrap);
    }
    this.#pool.length = 0;
  }

  #park(slots: Slots): void {
    this.#pool.push(slots);
    while (this.#pool.length > POOL_MAX) {
      const dropped = this.#pool.shift();
      if (dropped) {
        host.discard(dropped.wrap);
      }
    }
  }

  #create(): Slots {
    const wrap = host.transform();
    host.setPosition(wrap, true);
    // wick
    const wick = host.decorated([0, 0, 0, 255], 0);
    host.setPosition(wick, true);
    host.append(wrap, wick);
    // body
    const body = host.decorated([0, 0, 0, 255], 1);
    host.setPosition(body, true);
    host.append(wrap, body);
    // volume tick under the plot
    const vol = host.decorated([120, 128, 138, 255], 1);
    host.setPosition(vol, true);
    host.append(wrap, vol);
    return { wrap, wick, body, vol, index: -1 };
  }

  #paint(i: number, s: Slots, geom: ChartGeom): void {
    const bar = geom.bars[i];
    if (bar === undefined) {
      return;
    }
    const r = candleRects(bar, i, geom);
    s.index = i;
    host.setInset(s.wrap, i * geom.slotW, 0, 0, 0);
    const volH = Math.max(1, Math.min(geom.volH - 2, (bar.v / geom.maxVol) * (geom.volH - 2)));
    const volTop = geom.plotH + geom.volH - volH;
    host.setInset(s.vol, r.cx - 1, volTop, 0, 0);
    host.setMinSize(s.vol, 2, volH);
    host.setColor(s.vol, r.up ? [235, 62, 62, 120] : [34, 176, 95, 120]);
    if (r.bodyH <= 1.5 && Math.abs(r.bodyY - (r.bodyY + r.bodyH)) < 0.01) {
      // doji: thicken the body so it stays visible
      r.bodyY -= 0.75;
      r.bodyH = 1.5;
    }
    host.setInset(s.wick, r.cx - 0.5, r.wickY, 0, 0);
    host.setMinSize(s.wick, 1, r.wickH);
    host.setColor(s.wick, candleColor(r.up));
    host.setInset(s.body, r.cx - r.bodyW / 2, r.bodyY, 0, 0);
    host.setMinSize(s.body, r.bodyW, r.bodyH);
    host.setColor(s.body, candleColor(r.up));
  }
}
