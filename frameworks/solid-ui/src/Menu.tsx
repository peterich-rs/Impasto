import { onCleanup } from "solid-js";
import { host, onFrame, overlayParent, removeNode, type NodeHandle } from "@uifw/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./host-ext.ts";
import { moveChildren } from "./overlay.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal floating panel. A fill stack defers hits, so blank space reaches the content tree.
 * Does not push a focus scope and does not lay a scrim.
 */
function Floating(props: {
  open: boolean;
  onClose: () => void;
  place: (panel: number) => void;
  children?: NodeHandle;
}): NodeHandle {
  const theme = useTheme();
  let placeholder = 0;
  let shell = 0;
  let panel = 0;
  let slot = 0;
  let back = 0;

  function closeLayer(): void {
    if (slot !== 0 && placeholder !== 0) {
      moveChildren(slot, placeholder);
    }
    if (back !== 0) {
      host.offBack(back);
      back = 0;
    }
    if (shell !== 0) {
      removeNode({ id: shell } as NodeHandle);
    }
    shell = 0;
    panel = 0;
    slot = 0;
  }

  function ensure(): void {
    if (shell !== 0 || placeholder === 0) {
      return;
    }
    const overlay = nodeId(overlayParent());
    shell = host.stack();
    host.setFill(shell);
    host.insert(overlay, shell, 0);
    panel = host.transform();
    host.insert(shell, panel, 0);
    const paint = host.decorated(colorBytes(theme().color.surface), theme().radius.s);
    host.insert(panel, paint, 0);
    slot = host.column(theme().space.s);
    host.insert(paint, slot, 0);
    back = host.onBack(() => {
      props.onClose();
      return true;
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
    if (host.firstChild(placeholder) !== 0) {
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
    <clip
      min={{ w: 0, h: 0 }}
      max={{ w: 0, h: 0 }}
      ref={(node) => {
        placeholder = node.id;
      }}
    >
      {props.children}
    </clip>
  );
}

export function Menu(props: {
  open: boolean;
  x: number;
  y: number;
  onClose: () => void;
  children?: NodeHandle;
}): NodeHandle {
  return (
    <Floating
      open={props.open}
      onClose={props.onClose}
      place={(panel) => {
        host.setTransform(panel, props.x, props.y, 0, 1);
      }}
    >
      {props.children}
    </Floating>
  );
}

export function Popover(props: {
  open: boolean;
  anchor: number;
  onClose: () => void;
  children?: NodeHandle;
}): NodeHandle {
  return (
    <Floating
      open={props.open}
      onClose={props.onClose}
      place={(panel) => {
        const box = host.rect(props.anchor);
        host.setTransform(panel, box.x, box.y + box.h, 0, 1);
      }}
    >
      {props.children}
    </Floating>
  );
}
