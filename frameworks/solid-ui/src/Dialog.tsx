import { onCleanup } from "solid-js";
import { directionFor, host, onFrame, overlayParent, removeNode, restoreFocus, spring, takeInitialFocus, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { coreHost, nodeId } from "./host-ext.ts";
import { registerOverlay } from "./overlay/registry.ts";
import { moveChildren } from "./overlay.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A 0×0 placeholder. While open, children live in the overlay column.
 * Open and close fade through `host.animate`. The shell stays until the spring settles.
 */
export function Dialog(props: {
  open: boolean;
  onClose: () => void;
  title?: string;
  initialFocus?: number;
  children?: NodeHandle;
}): NodeHandle {
  const theme = useTheme();
  let placeholder = 0;
  let layer: Layer | null = null;
  let closing = false;
  let epoch = 0;

  function closeLayer(): void {
    if (layer === null) {
      return;
    }
    const current = layer;
    layer = null;
    closing = false;
    try {
      moveChildren(current.slot, placeholder);
      if (current.focused) {
        host.popFocusScope();
        restoreFocus(current.previous);
      }
    } finally {
      current.unregister?.();
      removeNode({ id: current.fade } as NodeHandle);
    }
  }

  function beginClose(): void {
    if (layer === null || closing) {
      return;
    }
    closing = true;
    layer.unregister?.();
    layer.unregister = null;
    const fade = layer.fade;
    const token = ++epoch;
    spring({
      id: fade,
      prop: "opacity",
      from: 255,
      to: 0,
      write: (value) => host.setAlpha(fade, Math.round(value)),
      onDone: () => {
        if (token === epoch) {
          closeLayer();
        }
      },
    });
  }

  function ensureLayer(): Layer | null {
    if (layer !== null) {
      return layer;
    }
    if (placeholder === 0) {
      return null;
    }
    const overlay = nodeId(overlayParent());
    const fade = host.opacity(0);
    host.setFill(fade);
    host.insert(overlay, fade, 0);
    const shell = host.stack();
    host.setFill(shell);
    host.insert(fade, shell, 0);

    const scrim = host.pointerRegion();
    host.setFill(scrim);
    host.onClick(scrim, () => props.onClose());
    host.insert(shell, scrim, 0);
    const paint = host.decorated(colorBytes(theme().color.scrim), 0);
    host.setFill(paint);
    host.insert(scrim, paint, 0);

    const frame = host.align("center");
    host.setFill(frame);
    host.insert(shell, frame, 0);
    const content = host.column(theme().space.m);
    host.insert(frame, content, 0);

    const title = props.title ?? "";
    let titleId = 0;
    if (title.length > 0) {
      titleId = host.text(title, theme().fontSize.title, colorBytes(theme().color.text));
      host.setFontWeight(titleId, theme().weight.emphasis);
      host.setTextDirection(titleId, directionFor(theme().locale));
      host.insert(content, titleId, 0);
    }
    host.setSemantics(shell, Role.Generic, title.length > 0 ? title : undefined);

    const slot = host.column(theme().space.s);
    host.insert(content, slot, 0);
    layer = { shell, slot, titleId, fade, previous: 0, back: 0, focused: false, unregister: null };
    spring({
      id: fade,
      prop: "opacity",
      from: 0,
      to: 255,
      write: (value) => host.setAlpha(fade, Math.round(value)),
    });
    return layer;
  }

  function sync(): void {
    if (placeholder === 0) {
      return;
    }
    if (!props.open) {
      beginClose();
      return;
    }
    if (closing) {
      epoch += 1;
      closing = false;
    }
    const current = ensureLayer();
    if (current === null) {
      return;
    }
    if (current.titleId !== 0) {
      host.setTextDirection(current.titleId, directionFor(theme().locale));
    }
    if (host.firstChild(placeholder) !== 0) {
      moveChildren(placeholder, current.slot);
    }
    if (current.focused) {
      return;
    }
    try {
      const previous = coreHost.focused();
      host.pushFocusScope(current.shell);
      current.previous = previous;
      current.unregister = registerOverlay({
        modal: true,
        close: () => props.onClose(),
      });
      current.focused = true;
      takeInitialFocus(current.shell, props.initialFocus);
    } catch {
      // A failed focus restore must not escape into runFrames.
    }
  }

  const stop = onFrame(() => {
    sync();
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
        host.setMinSize(node.id, 0, 0);
        host.setMaxSize(node.id, 0, 0);
      }}
    >
      {props.children}
    </clip>
  );
}

type Layer = {
  shell: number;
  slot: number;
  titleId: number;
  fade: number;
  previous: number;
  back: number;
  focused: boolean;
  unregister: (() => void) | null;
};
