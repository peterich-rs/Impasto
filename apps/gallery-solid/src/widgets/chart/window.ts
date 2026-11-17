import { scene } from "@impasto/solid-renderer";
import {
  candleColor,
  candleRects,
  visibleRange,
  type ChartGeom,
} from "./geometry.ts";
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
  update(
    scrollX: number,
    viewW: number,
    geom: ChartGeom,
    forceKey: string,
  ): number {
    const key = forceKey;
    if (key === this.#lastKey && this.#mounted.size > 0) {
      return 0;
    }
    this.#lastKey = key;
    const [first, last] = visibleRange(scrollX, viewW, geom);
    let writes = 0;
    for (const [i, slots] of this.#mounted) {
      if (i < first || i > last) {
        scene.detach(this.#parent, slots.wrap);
        this.#park(slots);
        this.#mounted.delete(i);
      }
    }
    for (let i = first; i <= last; i++) {
      let slots = this.#mounted.get(i);
      if (slots === undefined) {
        slots = this.#pool.pop() ?? this.#create();
        scene.append(this.#parent, slots.wrap);
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
      scene.discard(slots.wrap);
    }
    this.#mounted.clear();
    for (const slots of this.#pool) {
      scene.discard(slots.wrap);
    }
    this.#pool.length = 0;
  }
  #park(slots: Slots): void {
    this.#pool.push(slots);
    while (this.#pool.length > POOL_MAX) {
      const dropped = this.#pool.shift();
      if (dropped) {
        scene.discard(dropped.wrap);
      }
    }
  }
  #create(): Slots {
    const wrap = scene.createElement("stack", {
      style: {},
    });
    scene.patchStyle(wrap, {
      position: "absolute",
    });
    // wick
    const wick = scene.createElement("view", {
      style: {
        backgroundColor: [0, 0, 0, 255],
        borderRadius: 0,
      },
    });
    scene.patchStyle(wick, {
      position: "absolute",
    });
    scene.append(wrap, wick);
    // body
    const body = scene.createElement("view", {
      style: {
        backgroundColor: [0, 0, 0, 255],
        borderRadius: 1,
      },
    });
    scene.patchStyle(body, {
      position: "absolute",
    });
    scene.append(wrap, body);
    // volume tick under the plot
    const vol = scene.createElement("view", {
      style: {
        backgroundColor: [120, 128, 138, 255],
        borderRadius: 1,
      },
    });
    scene.patchStyle(vol, {
      position: "absolute",
    });
    scene.append(wrap, vol);
    return {
      wrap,
      wick,
      body,
      vol,
      index: -1,
    };
  }
  #paint(i: number, s: Slots, geom: ChartGeom): void {
    const bar = geom.bars[i];
    if (bar === undefined) {
      return;
    }
    const r = candleRects(bar, i, geom);
    s.index = i;
    scene.patchStyle(s.wrap, {
      top: i * geom.slotW,
      right: 0,
      bottom: 0,
      left: 0,
    });
    const volH = Math.max(
      1,
      Math.min(geom.volH - 2, (bar.v / geom.maxVol) * (geom.volH - 2)),
    );
    const volTop = geom.plotH + geom.volH - volH;
    scene.patchStyle(s.vol, {
      top: r.cx - 1,
      right: volTop,
      bottom: 0,
      left: 0,
    });
    scene.patchStyle(s.vol, {
      minWidth: 2,
      minHeight: volH,
    });
    scene.patchStyle(s.vol, {
      backgroundColor: r.up ? [235, 62, 62, 120] : [34, 176, 95, 120],
    });
    if (r.bodyH <= 1.5 && Math.abs(r.bodyY - (r.bodyY + r.bodyH)) < 0.01) {
      // doji: thicken the body so it stays visible
      r.bodyY -= 0.75;
      r.bodyH = 1.5;
    }
    scene.patchStyle(s.wick, {
      top: r.cx - 0.5,
      right: r.wickY,
      bottom: 0,
      left: 0,
    });
    scene.patchStyle(s.wick, {
      minWidth: 1,
      minHeight: r.wickH,
    });
    scene.patchStyle(s.wick, {
      backgroundColor: candleColor(r.up),
    });
    scene.patchStyle(s.body, {
      top: r.cx - r.bodyW / 2,
      right: r.bodyY,
      bottom: 0,
      left: 0,
    });
    scene.patchStyle(s.body, {
      minWidth: r.bodyW,
      minHeight: r.bodyH,
    });
    scene.patchStyle(s.body, {
      backgroundColor: candleColor(r.up),
    });
  }
}
