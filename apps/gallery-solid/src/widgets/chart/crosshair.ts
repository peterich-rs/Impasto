import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import { fmtPrice } from "../../format.ts";

const WHITE = colorBytes("#FFFFFF");
const PANEL = colorBytes("#2A2F3A");
const HAIR = colorBytes("#E8E8EC");

export type CrosshairState = {
  show(x: number, y: number): void;
  hide(): void;
};

/**
 * Crosshair overlay: 1px hairlines plus a price bubble on the right edge and
 * a time bubble on the bottom. Driven by press-drag on the chart viewport:
 * press shows it, release returns to scrolling.
 */
export function Crosshair(props: {
  plotW: () => number;
  plotH: () => number;
  priceAt: (y: number) => number;
  labelAt: (x: number) => string;
}): NodeHandle {
  const root = host.stack();
  host.setPosition(root, true);
  host.setInset(root, 0, 0, 0, 0);

  const vLine = fadedPaint(HAIR, 0);
  const hLine = fadedPaint(HAIR, 0);
  const bubble = fadedPaint(PANEL, 4);
  const bubbleText = host.text("", 11, WHITE);
  host.append(bubble.paint, bubbleText);
  const timeBubble = fadedPaint(PANEL, 4);
  const timeText = host.text("", 11, WHITE);
  host.append(timeBubble.paint, timeText);

  function show(x: number, y: number): void {
    const w = props.plotW();
    const h = props.plotH();
    const cx = Math.max(0, Math.min(w, x));
    const cy = Math.max(0, Math.min(h, y));
    host.setAlpha(vLine.fade, 255);
    host.setAlpha(hLine.fade, 255);
    host.setAlpha(bubble.fade, 255);
    host.setAlpha(timeBubble.fade, 255);
    host.setInset(vLine.fade, 0, 0, 0, Math.round(cx));
    host.setMinSize(vLine.fade, 1, Math.round(h));
    host.setInset(hLine.fade, 0, Math.round(cy), 0, 0);
    host.setMinSize(hLine.fade, Math.round(w), 1);
    host.setInset(bubble.fade, 0, Math.round(cy) - 10, 0, Math.round(w) - 56);
    host.setText(bubbleText, fmtPrice(props.priceAt(cy)));
    host.setInset(timeBubble.fade, 0, Math.round(h) + 2, 0, Math.round(cx) - 32);
    host.setText(timeText, props.labelAt(cx));
  }

  function hide(): void {
    host.setAlpha(vLine.fade, 0);
    host.setAlpha(hLine.fade, 0);
    host.setAlpha(bubble.fade, 0);
    host.setAlpha(timeBubble.fade, 0);
  }

  function fadedPaint(color: number[], radius: number): { fade: number; paint: number } {
    const fade = host.opacity(0);
    host.setPosition(fade, true);
    const paint = host.decorated(color, radius);
    host.setFill(paint);
    host.append(fade, paint);
    host.append(root, fade);
    return { fade, paint };
  }

  return { id: root, show, hide };
}
