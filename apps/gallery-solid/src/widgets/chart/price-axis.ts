import { scene, type NodeHandle } from "@impasto/solid-renderer";
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
}) {
  const root = scene.stack();
  scene.patchStyle(root, {
    position: "absolute",
  });
  scene.patchStyle(root, {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
  });
  scene.patchStyle(root, {
    minWidth: 56,
    minHeight: 0,
  });
  const rows: {
    line: number;
    label: number;
  }[] = [];
  for (let i = 0; i <= TICKS; i++) {
    const line = scene.createElement("view", {
      style: {
        backgroundColor: GRID,
        borderRadius: 0,
      },
    });
    scene.patchStyle(line, {
      minWidth: 0,
      minHeight: 1,
    });
    scene.patchStyle(line, {
      position: "absolute",
    });
    scene.append(root, line);
    const label = scene.createElement("text", {
      value: "—",
      style: {
        fontSize: 10,
        color: MUTED,
      },
    });
    scene.append(root, label);
    rows.push({
      line,
      label,
    });
  }
  function refresh(geom: ChartGeom, slotCount: number): void {
    const h = geom.plotH;
    for (let i = 0; i <= TICKS; i++) {
      const price = geom.hi - ((geom.hi - geom.lo) * i) / TICKS;
      const y = (h * i) / TICKS;
      const row = rows[i]!;
      scene.patchStyle(row.line, {
        minWidth: slotCount * geom.slotW,
        minHeight: 1,
      });
      scene.patchStyle(row.line, {
        top: 0,
        right: Math.round(y),
        bottom: 0,
        left: 0,
      });
      scene.patchStyle(row.label, {
        top: 0,
        right: Math.round(y) - 7,
        bottom: 0,
        left: 0,
      });
      scene.setText(row.label, price.toFixed(2));
    }
  }
  refresh(props.geom(), props.slotCount());
  void INK;
  void HAIR;
  return {
    node: root,
    refresh,
  };
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
