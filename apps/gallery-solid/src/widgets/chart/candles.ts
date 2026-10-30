import { createSignal, onCleanup } from "solid-js";
import { host, onFrame, requestFrame, type NodeHandle } from "@impasto/solid-renderer";
import { ScrollArea } from "@impasto/solid-ui";
import type { ScrollController } from "@impasto/solid-ui";
import { colorBytes } from "../../colors.ts";
import { fmtVolumeCN } from "../../format.ts";
import type { QuoteEngine } from "../../../engine/engine.ts";
import { geomFor, yOf, type ChartGeom } from "./geometry.ts";
import { ChartWindow } from "./window.ts";
import { PriceAxis } from "./price-axis.ts";
import { Crosshair } from "./crosshair.ts";

const HAIR = colorBytes("#E8E8EC");

const SLOT_MIN = 2;
const SLOT_MAX = 24;

/**
 * Windowed candle chart on a horizontal scroll. The content node is sized to
 * `bars * slotW`; the ChartWindow mounts only the candles near the viewport.
 * Press-drag shows the crosshair and pauses scrolling; release resumes it.
 * Zoom changes `slotW` around the viewport's center bar.
 */
export function CandleChart(props: {
  symbol: () => string;
  engine: () => QuoteEngine;
  controller: ScrollController;
  plotH: number;
  volH: number;
  viewW: () => number;
  barLabel?: (barIndex: number) => string;
}): NodeHandle {
  const engine = props.engine;
  const symbol = props.symbol;
  const [slotW, setSlotW] = createSignal(8);
  const win = new ChartWindow();
  let content = 0;
  let viewport = 0;
  let lastAxisKey = "";

  const totalH = props.plotH + props.volH;

  const root = host.stack();
  host.setMinSize(root, 0, totalH);
  host.setStroke(root, 1, HAIR);

  const scrollArea = ScrollArea({
    axis: "horizontal",
    controller: props.controller,
    viewportRef: (id) => {
      viewport = id;
    },
    contentRef: (id) => {
      content = id;
      win.attach(id);
      resizeContent();
    },
    children: [],
  });
  host.append(root, scrollArea.id);

  // Initial anchor: the newest bar at the right edge.
  let anchored = false;

  const axis = PriceAxis({ geom: () => geomNow(), slotCount: () => Math.ceil(props.viewW() / slotW()) + 1 });
  host.setPosition(axis.id, true);
  host.setInset(axis.id, 0, 0, 0, 0);
  host.append(root, axis.id);

  const crosshair = Crosshair({
    plotW: () => Math.min(props.viewW(), barsNow().length * slotW()),
    plotH: () => props.plotH,
    priceAt: (y) => {
      const g = geomNow();
      return g.hi - ((g.hi - g.lo) * y) / g.plotH;
    },
    labelAt: (x) => {
      const off = props.controller.offset().x;
      const index = Math.floor((off + x) / slotW());
      return props.barLabel?.(index) ?? String(index);
    },
  });
  host.append(root, crosshair.id);

  function barsNow() {
    return engine().bars(symbol());
  }
  function geomNow(): ChartGeom {
    const bars = barsNow();
    const off = props.controller.offset().x;
    const first = Math.max(0, Math.floor(off / slotW()) - 12);
    const last = Math.min(bars.length - 1, Math.ceil((off + props.viewW()) / slotW()) + 12);
    return geomFor(bars, slotW(), props.plotH, props.volH, first, last);
  }

  function resizeContent(): void {
    if (content !== 0) {
      host.setMinSize(content, barsNow().length * slotW(), totalH);
    }
  }

  // Crosshair gestures ride the scroll viewport: press shows it and stops the
  // scroll, drag moves it, release hides it and lets scrolling resume.
  host.onPress(viewport, () => {
    props.controller.stop();
    const p = host.pointerPosition();
    crosshair.show(p.x, p.y);
  });
  host.onDragBy(viewport, (dx, dy) => {
    const p = host.pointerPosition();
    crosshair.show(p.x, p.y);
    void dx;
    void dy;
  });
  host.onRelease(viewport, () => {
    crosshair.hide();
  });
  host.onZoom(viewport, (factor) => {
    zoom(factor);
  });

  function zoom(factor: number): void {
    const off = props.controller.offset().x;
    const centerBar = (off + props.viewW() / 2) / slotW();
    const next = Math.max(SLOT_MIN, Math.min(SLOT_MAX, Math.round(slotW() * factor)));
    if (next === slotW()) {
      return;
    }
    setSlotW(next);
    resizeContent();
    const target = Math.max(0, centerBar * next - props.viewW() / 2);
    props.controller.scrollTo(target, 0);
  }

  let lastKey = "";
  const stopFrame = onFrame(() => {
    const bars = barsNow();
    if (bars.length === 0) {
      return;
    }
    if (!anchored) {
      anchored = true;
      resizeContent();
      props.controller.scrollTo(Math.max(0, bars.length * slotW() - props.viewW()), 0);
    }
    const off = props.controller.offset().x;
    const first = Math.max(0, Math.floor(off / slotW()) - 12);
    const last = Math.min(bars.length - 1, Math.ceil((off + props.viewW()) / slotW()) + 12);
    const g = geomFor(bars, slotW(), props.plotH, props.volH, first, last);
    const q = engine().quote(symbol());
    const key = `${off}|${slotW()}|${props.viewW()}|${q.revision}|${bars.length}`;
    win.update(off, props.viewW(), g, key);
    const axisKey = `${g.lo}|${g.hi}|${props.viewW()}`;
    if (axisKey !== lastAxisKey) {
      lastAxisKey = axisKey;
      axis.refresh(g, Math.ceil(props.viewW() / slotW()) + 1);
    }
    if (key !== lastKey) {
      lastKey = key;
      requestFrame();
    }
  });
  onCleanup(stopFrame);

  void fmtVolumeCN;
  void yOf;
  return { id: root };
}
