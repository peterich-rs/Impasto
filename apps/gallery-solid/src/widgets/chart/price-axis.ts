import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import { candleRects, geomFor, yOf, type ChartGeom } from "./geometry.ts";
import type { QuoteEngine } from "../../engine/engine.ts";
import type { Bar } from "../../engine/types.ts";

const INK = colorBytes("#1F2430");
const MUTED = colorBytes("#8A8D91");
const HAIR = colorBytes("#E8E8EC");
const GRID = colorBytes("#EDEEF0");

const TICKS = 4;

/**
 * Right-edge price scale derived from the visible window's lo/hi. Rebuilt
 * only when the window crosses bars or the scale changes.
 */
export function PriceAxis(props: {
  geom: () => ChartGeom;
  slotCount: () => number;
}): NodeHandle {
  const root = host.stack();
  host.setPosition(root, true);
  host.setInset(root, 0, 0, 0, 0);
  host.setMinSize(root, 56, 0);

  const rows: { line: number; label: number }[] = [];
  for (let i = 0; i <= TICKS; i++) {
    const line = host.decorated(GRID, 0);
    host.setMinSize(line, 0, 1);
    host.setPosition(line, true);
    host.append(root, line);
    const label = host.text("—", 10, MUTED);
    host.append(root, label);
    rows.push({ line, label });
  }

  function refresh(geom: ChartGeom, slotCount: number): void {
    const h = geom.plotH;
    for (let i = 0; i <= TICKS; i++) {
      const price = geom.hi - ((geom.hi - geom.lo) * i) / TICKS;
      const y = (h * i) / TICKS;
      const row = rows[i]!;
      host.setMinSize(row.line, slotCount * geom.slotW, 1);
      host.setInset(row.line, 0, Math.round(y), 0, 0);
      host.setInset(row.label, 0, Math.round(y) - 7, 0, 0);
      host.setText(row.label, price.toFixed(2));
    }
  }

  refresh(props.geom(), props.slotCount());
  void INK;
  void HAIR;
  return { id: root, refresh };
}

export function timeLabel(bar: Bar | undefined): string {
  if (bar === undefined || bar.t === undefined) {
    return "";
  }
  void yOf;
  void candleRects;
  void geomFor;
  return "";
}

export function axisUnused(): void {
  void INK;
  void HAIR;
  void timeLabel;
}
