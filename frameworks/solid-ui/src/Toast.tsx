import { onCleanup } from "solid-js";
import { host, onFrame, overlayParent, removeNode, requestFrame, type NodeHandle } from "@uifw/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./host-ext.ts";
import { directionFor } from "./i18n.ts";
import { transition } from "./motion.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal message. No scrim and no focus scope. Closes after `durationMs`.
 * Leave runs in reverse and removes the shell only after it finishes.
 */
export function Toast(props: {
  open: boolean;
  text: string;
  durationMs?: number;
  onClose: () => void;
}): NodeHandle {
  const theme = useTheme();
  let shell = 0;
  let panel = 0;
  let fade = 0;
  let label = 0;
  let openedAt: number | null = null;
  let closing = false;
  let stopSlide = () => {};

  function paint(t: number, leaving: boolean): void {
    if (fade === 0 || panel === 0) {
      return;
    }
    const shown = leaving ? 1 - t : t;
    host.setAlpha(fade, Math.round(shown * 255));
    host.setTransform(panel, 0, (1 - shown) * 24, 0, 1);
  }

  function dismiss(): void {
    stopSlide();
    if (shell !== 0) {
      removeNode({ id: shell } as NodeHandle);
    }
    shell = 0;
    panel = 0;
    fade = 0;
    label = 0;
    openedAt = null;
    closing = false;
  }

  function ensure(): void {
    if (shell !== 0) {
      return;
    }
    const overlay = nodeId(overlayParent());
    shell = host.stack();
    host.setFill(shell);
    host.setAlign(shell, "end");
    host.insert(overlay, shell, 0);
    const pad = host.padding(theme().space.l);
    host.insert(shell, pad, 0);
    fade = host.opacity(0);
    host.insert(pad, fade, 0);
    panel = host.transform();
    host.insert(fade, panel, 0);
    const paintNode = host.decorated(colorBytes(theme().color.surface), theme().radius.m);
    host.insert(panel, paintNode, 0);
    const inner = host.padding(theme().space.m);
    host.insert(paintNode, inner, 0);
    label = host.text(props.text, theme().fontSize.body, colorBytes(theme().color.text));
    host.setFontWeight(label, theme().weight.body);
    host.setTextDirection(label, directionFor(theme().locale));
    host.insert(inner, label, 0);
  }

  function beginClose(): void {
    if (shell === 0 || closing) {
      return;
    }
    closing = true;
    if (theme().reduceMotion) {
      dismiss();
      return;
    }
    stopSlide();
    stopSlide = transition((t) => {
      paint(t, true);
      if (t >= 1) {
        dismiss();
      }
    }, 180);
  }

  function sync(targetMs: number): void {
    if (props.open) {
      if (closing) {
        stopSlide();
        closing = false;
      }
      const born = shell === 0;
      ensure();
      if (label !== 0) {
        host.setText(label, props.text);
        host.setTextDirection(label, directionFor(theme().locale));
      }
      if (born) {
        openedAt = targetMs;
        stopSlide();
        stopSlide = transition((t) => paint(t, false), theme().reduceMotion ? 0 : 180);
      }
      const duration = props.durationMs ?? 3000;
      if (openedAt !== null && targetMs - openedAt >= duration) {
        props.onClose();
        requestFrame();
        return;
      }
      requestFrame();
      return;
    }
    beginClose();
  }

  const stop = onFrame((targetMs) => {
    try {
      sync(targetMs);
    } catch {
      // Keep the listener. Closing still removes the shell on a later frame.
    }
  });
  onCleanup(() => {
    stop();
    stopSlide();
    dismiss();
  });

  return <clip min={{ w: 0, h: 0 }} max={{ w: 0, h: 0 }} />;
}
