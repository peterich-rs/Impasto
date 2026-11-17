import { onCleanup } from "solid-js";
import {
  scene,
  onFrame,
  overlayParent,
  removeNode,
  spring,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./scene-ext.ts";
import { moveChildren, readFirstChild, restoreChildren } from "./overlay.ts";
import type { MenuProps } from "./props.ts";
import { createNodeRef, createStyleTarget } from "./target.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal floating panel. A fill stack defers hits, so blank space reaches the content tree.
 * Does not push a focus scope and does not lay a scrim.
 */
function Floating(props: MenuProps & { place: (panel: number) => void }): NodeHandle {
  const theme = useTheme();
  const panelStyle = createStyleTarget(() => props, () => ({
    backgroundColor: colorBytes(theme().color.surface), borderRadius: theme().radius.s,
  }));
  const contentStyle = createStyleTarget(() => props.contentProps, () => ({ gap: theme().space.s }));
  const panelRef = createNodeRef(() => props.ref);
  let placeholder = 0;
  let shell = 0;
  let fade = 0;
  let panel = 0;
  let slot = 0;
  function closeLayer(): void {
    panelRef.clear();
    panelStyle.detach();
    contentStyle.detach();
    restoreChildren(slot, placeholder);
    if (shell !== 0) {
      removeNode(shell);
    }
    shell = 0;
    fade = 0;
    panel = 0;
    slot = 0;
  }
  function ensure(): void {
    if (shell !== 0 || placeholder === 0) {
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
    slot = scene.createElement("view");
    scene.insert(panel, slot, 0);
    panelStyle.attach(panel);
    contentStyle.attach(slot);
    panelRef.attach(panel);
    spring({
      id: fade,
      prop: "opacity",
      from: 0,
      to: 255,
      write: (value) =>
        scene.patchStyle(fade, {
          opacity: Math.round(value) / 255,
        }),
    });
  }
  function sync(): void {
    if (placeholder === 0) {
      return;
    }
    if (!props.open) {
      closeLayer();
      return;
    }
    ensure();
    if (shell === 0) {
      return;
    }
    const parked = readFirstChild(placeholder);
    if (parked === null) {
      placeholder = 0;
      closeLayer();
      return;
    }
    if (parked !== 0) {
      moveChildren(placeholder, slot);
    }
    props.place(panel);
  }
  const stop = onFrame(() => {
    try {
      sync();
    } catch {
      // Keep the listener if placement throws.
    }
  });
  onCleanup(() => {
    stop();
    closeLayer();
  });
  return (
    <stack
      ref={(node) => {
        if (node === null) return;
        placeholder = node;
      }}
      style={{
        overflow: "clip",
        minWidth: 0,
        minHeight: 0,
        maxWidth: 0,
        maxHeight: 0,
      }}
    >
      {props.children}
    </stack>
  );
}
export function Menu(props: MenuProps): NodeHandle {
  return (
    <Floating
      {...props}
      place={(panel) => {
        scene.placePanel(panel, props.anchor, 0);
      }}
    >
      {props.children}
    </Floating>
  );
}
export function Popover(props: MenuProps): NodeHandle {
  return (
    <Floating
      {...props}
      place={(panel) => {
        scene.placePanel(panel, props.anchor, 0);
      }}
    >
      {props.children}
    </Floating>
  );
}
