import { onCleanup } from "solid-js";
import {
  directionFor,
  scene,
  onFrame,
  overlayParent,
  removeNode,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./scene-ext.ts";
import { transition } from "./motion.ts";
import type { TooltipProps } from "./props.ts";
import { createNodeRef, createStyleTarget } from "./target.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal hint. Positioned with `patchStyle` translate from the anchor's `rect`.
 * No scrim and no focus scope, so a blank click falls through to the content tree.
 */
export function Tooltip(props: TooltipProps): NodeHandle {
  const theme = useTheme();
  const panelStyle = createStyleTarget(() => props, () => ({
    backgroundColor: colorBytes(theme().color.surface), borderRadius: theme().radius.s,
  }));
  const textStyle = createStyleTarget(() => props.textProps, () => ({
    fontSize: theme().fontSize.body, color: colorBytes(theme().color.text),
    fontWeight: theme().weight.body, writingDirection: directionFor(theme().locale),
  }));
  const panelRef = createNodeRef(() => props.ref);
  let shell = 0;
  let fade = 0;
  let panel = 0;
  let label = 0;
  let stopFade = () => {};
  function dismiss(): void {
    panelRef.clear();
    panelStyle.detach();
    textStyle.detach();
    stopFade();
    if (shell !== 0) {
      removeNode(shell);
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
    shell = scene.stack();
    scene.setComponentStyle(shell, {
      gridTemplateColumns: "minmax(0, 1fr)",
      gridTemplateRows: "minmax(0, 1fr)",
    });
    scene.patchStyle(shell, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.insert(overlay, shell, 0);
    fade = scene.createElement("stack", {
      style: {
        opacity: 0 / 255,
      },
    });
    scene.insert(shell, fade, 0);
    panel = scene.createElement("stack", {
      style: {},
    });
    scene.insert(fade, panel, 0);
    const pad = scene.createElement("view", {
      style: {
        padding: theme().space.s,
      },
    });
    scene.insert(panel, pad, 0);
    label = scene.createElement("text", { value: props.text });
    scene.insert(pad, label, 0);
    panelStyle.attach(panel);
    textStyle.attach(label);
    panelRef.attach(panel);
    stopFade();
    stopFade = transition((t) => {
      scene.patchStyle(fade, {
        opacity: Math.round(t * 255) / 255,
      });
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
    scene.setText(label, props.text);
    scene.placePanel(panel, props.anchor, 0);
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
  return (
    <stack
      style={{
        overflow: "clip",
        minWidth: 0,
        minHeight: 0,
        maxWidth: 0,
        maxHeight: 0,
      }}
    />
  );
}
