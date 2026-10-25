import { onCleanup } from "solid-js";
import { host, onFrame, overlayParent, removeNode, type NodeHandle } from "@uifw/solid-renderer";
import { colorBytes } from "./colors.ts";
import { coreHost, nodeId } from "./host-ext.ts";
import { directionFor } from "./i18n.ts";
import { moveChildren, restoreFocus, takeInitialFocus } from "./overlay.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A 0×0 placeholder. While open, children live in the overlay column.
 * The move and the close sequence run on the runtime frame listener. No animation.
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

  function closeLayer(): void {
    if (layer === null) {
      return;
    }
    const current = layer;
    layer = null;
    try {
      moveChildren(current.slot, placeholder);
      if (current.focused) {
        host.popFocusScope();
        restoreFocus(current.previous);
      }
    } finally {
      if (current.back !== 0) {
        host.offBack(current.back);
      }
      removeNode({ id: current.shell } as NodeHandle);
    }
  }

  function ensureLayer(): Layer | null {
    if (layer !== null) {
      return layer;
    }
    if (placeholder === 0) {
      return null;
    }
    const overlay = nodeId(overlayParent());
    const shell = host.stack();
    host.setFill(shell);
    host.insert(overlay, shell, 0);

    const scrim = host.pointerRegion();
    host.setFill(scrim);
    host.onClick(scrim, () => props.onClose());
    host.insert(shell, scrim, 0);
    const paint = host.decorated(colorBytes(theme().color.scrim), 0);
    host.setFill(paint);
    host.insert(scrim, paint, 0);

    const content = host.column(theme().space.m);
    host.setFill(content);
    host.setAlign(content, "center");
    host.insert(shell, content, 0);

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
    layer = { shell, slot, titleId, previous: 0, back: 0, focused: false };
    return layer;
  }

  function sync(): void {
    if (placeholder === 0) {
      return;
    }
    if (!props.open) {
      closeLayer();
      return;
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
    const previous = coreHost.focused();
    host.pushFocusScope(current.shell);
    current.previous = previous;
    current.back = host.onBack(() => {
      props.onClose();
      return true;
    });
    current.focused = true;
    takeInitialFocus(current.shell, props.initialFocus);
  }

  const stop = onFrame(() => {
    try {
      sync();
    } catch {
      // A failed focus restore must not escape into runFrames.
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
  previous: number;
  back: number;
  focused: boolean;
};
