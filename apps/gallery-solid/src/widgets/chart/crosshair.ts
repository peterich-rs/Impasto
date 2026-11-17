import { scene, type NodeHandle } from "@impasto/solid-renderer";
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
  const vLine = fadedPaint(HAIR, 0);
  const hLine = fadedPaint(HAIR, 0);
  const bubble = fadedPaint(PANEL, 4);
  const bubbleText = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 11,
      color: WHITE,
    },
  });
  scene.append(bubble.paint, bubbleText);
  const timeBubble = fadedPaint(PANEL, 4);
  const timeText = scene.createElement("text", {
    value: "",
    style: {
      fontSize: 11,
      color: WHITE,
    },
  });
  scene.append(timeBubble.paint, timeText);
  function show(x: number, y: number): void {
    const w = props.plotW();
    const h = props.plotH();
    const cx = Math.max(0, Math.min(w, x));
    const cy = Math.max(0, Math.min(h, y));
    scene.patchStyle(vLine.fade, {
      opacity: 255 / 255,
    });
    scene.patchStyle(hLine.fade, {
      opacity: 255 / 255,
    });
    scene.patchStyle(bubble.fade, {
      opacity: 255 / 255,
    });
    scene.patchStyle(timeBubble.fade, {
      opacity: 255 / 255,
    });
    scene.patchStyle(vLine.fade, {
      top: 0,
      right: 0,
      bottom: 0,
      left: Math.round(cx),
    });
    scene.patchStyle(vLine.fade, {
      minWidth: 1,
      minHeight: Math.round(h),
    });
    scene.patchStyle(hLine.fade, {
      top: 0,
      right: Math.round(cy),
      bottom: 0,
      left: 0,
    });
    scene.patchStyle(hLine.fade, {
      minWidth: Math.round(w),
      minHeight: 1,
    });
    scene.patchStyle(bubble.fade, {
      top: 0,
      right: Math.round(cy) - 10,
      bottom: 0,
      left: Math.round(w) - 56,
    });
    scene.setText(bubbleText, fmtPrice(props.priceAt(cy)));
    scene.patchStyle(timeBubble.fade, {
      top: 0,
      right: Math.round(h) + 2,
      bottom: 0,
      left: Math.round(cx) - 32,
    });
    scene.setText(timeText, props.labelAt(cx));
  }
  function hide(): void {
    scene.patchStyle(vLine.fade, {
      opacity: 0 / 255,
    });
    scene.patchStyle(hLine.fade, {
      opacity: 0 / 255,
    });
    scene.patchStyle(bubble.fade, {
      opacity: 0 / 255,
    });
    scene.patchStyle(timeBubble.fade, {
      opacity: 0 / 255,
    });
  }
  function fadedPaint(
    color: number[],
    radius: number,
  ): {
    fade: number;
    paint: number;
  } {
    const fade = scene.createElement("stack", {
      style: {
        opacity: 0 / 255,
      },
    });
    scene.patchStyle(fade, {
      position: "absolute",
    });
    const paint = scene.createElement("view", {
      style: {
        backgroundColor: color,
        borderRadius: radius,
      },
    });
    scene.patchStyle(paint, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.append(fade, paint);
    scene.append(root, fade);
    return {
      fade,
      paint,
    };
  }
  return {
    node: root,
    show,
    hide,
  };
}
