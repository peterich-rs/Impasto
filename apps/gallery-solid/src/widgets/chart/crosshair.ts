import { host, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../../colors.ts";
import { fmtPrice } from "../../format.ts";

const INK = colorBytes("#1F2430");
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

  const vLine = host.decorated(HAIR, 0);
  host.setMinSize(vLine, 1, 0);
  host.setPosition(vLine, true);
  host.setAlpha(vLine, 0);
  host.append(root, vLine);

  const hLine = host.decorated(HAIR, 0);
  host.setMinSize(hLine, 0, 1);
  host.setPosition(hLine, true);
  host.setAlpha(hLine, 0);
  host.append(root, hLine);

  const bubble = host.decorated(PANEL, 4);
  host.setMinSize(bubble, 58, 20);
  host.setPosition(bubble, true);
  host.setAlpha(bubble, 0);
  host.append(root, bubble);
  const bubbleText = host.text("", 11, WHITE);
  host.append(bubble, bubbleText);

  const timeBubble = host.decorated(PANEL, 4);
  host.setMinSize(timeBubble, 64, 20);
  host.setPosition(timeBubble, true);
  host.setAlpha(timeBubble, 0);
  host.append(root, timeBubble);
  const timeText = host.text("", 11, WHITE);
  host.append(timeBubble, timeText);

  function show(x: number, y: number): void {
    const w = props.plotW();
    const h = props.plotH();
    const cx = Math.max(0, Math.min(w, x));
    const cy = Math.max(0, Math.min(h, y));
    host.setAlpha(vLine, 255);
    host.setAlpha(hLine, 255);
    host.setAlpha(bubble, 255);
    host.setAlpha(timeBubble, 255);
    host.setInset(vLine, 0, 0, 0, Math.round(cx));
    host.setMinSize(vLine, 1, Math.round(h));
    host.setInset(hLine, 0, Math.round(cy), 0, 0);
    host.setMinSize(hLine, Math.round(w), 1);
    host.setInset(bubble, 0, Math.round(cy) - 10, 0, Math.round(w) - 56);
    host.setText(bubbleText, fmtPrice(props.priceAt(cy)));
    host.setInset(timeBubble, 0, Math.round(h) + 2, 0, Math.round(cx) - 32);
    host.setText(timeText, props.labelAt(cx));
  }

  function hide(): void {
    host.setAlpha(vLine, 0);
    host.setAlpha(hLine, 0);
    host.setAlpha(bubble, 0);
    host.setAlpha(timeBubble, 0);
  }

  return { id: root, show, hide };
}
