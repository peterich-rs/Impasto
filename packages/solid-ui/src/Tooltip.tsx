import { onCleanup } from "solid-js";
import { directionFor, host, onFrame, overlayParent, removeNode, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./host-ext.ts";
import { transition } from "./motion.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal hint. Positioned with `setTransform` from the anchor's `rect`.
 * No scrim and no focus scope, so a blank click falls through to the content tree.
 */
export function Tooltip(props: { open: boolean; anchor: number; text: string }): NodeHandle {
  const theme = useTheme();
  let shell = 0;
  let fade = 0;
  let panel = 0;
  let label = 0;
  let stopFade = () => {};

  function dismiss(): void {
    stopFade();
    if (shell !== 0) {
      removeNode({ id: shell } as NodeHandle);
    }
    shell = 0;
    fade = 0;
    panel = 0;
    label = 0;
  }

  function ensure(): void {
    if (shell !== 0) {
      return;
    }
    const overlay = nodeId(overlayParent());
    shell = host.stack();
    host.setFill(shell);
    host.insert(overlay, shell, 0);
    fade = host.opacity(0);
    host.insert(shell, fade, 0);
    panel = host.transform();
    host.insert(fade, panel, 0);
    const paint = host.decorated(colorBytes(theme().color.surface), theme().radius.s);
    host.insert(panel, paint, 0);
    const pad = host.padding(theme().space.s);
    host.insert(paint, pad, 0);
    label = host.text(props.text, theme().fontSize.body, colorBytes(theme().color.text));
    host.setTextDirection(label, directionFor(theme().locale));
    host.insert(pad, label, 0);
    stopFade();
    stopFade = transition((t) => {
      host.setAlpha(fade, Math.round(t * 255));
    }, 80);
  }

  function sync(): void {
    if (!props.open) {
      dismiss();
      return;
    }
    ensure();
    if (shell === 0 || label === 0) {
      return;
    }
    host.setText(label, props.text);
    host.setTextDirection(label, directionFor(theme().locale));
    const box = host.rect(props.anchor);
    host.setTransform(panel, box.x, box.y + box.h, 0, 1);
  }

  const stop = onFrame(() => {
    try {
      sync();
    } catch {
      // Keep the listener if the anchor rect is not ready.
    }
  });
  onCleanup(() => {
    stop();
    stopFade();
    dismiss();
  });

  return <clip min={{ w: 0, h: 0 }} max={{ w: 0, h: 0 }} />;
}
