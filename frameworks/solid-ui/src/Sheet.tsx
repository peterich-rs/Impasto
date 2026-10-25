import { onCleanup } from "solid-js";
import { host, onFrame, overlayParent, removeNode, type NodeHandle } from "@uifw/solid-renderer";
import { colorBytes } from "./colors.ts";
import { coreHost, nodeId } from "./host-ext.ts";
import { directionFor } from "./i18n.ts";
import { moveChildren, restoreFocus, takeInitialFocus } from "./overlay.ts";
import { transition } from "./motion.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Bottom sheet with a scrim and a focus scope. Children park in a 0×0 clip.
 * Enter and leave slide; reduce-motion skips the in-between frames.
 */
export function Sheet(props: {
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
  let stopSlide = () => {};

  function closeLayer(): void {
    stopSlide();
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
      if (current.back !== 0) {
        host.offBack(current.back);
      }
      removeNode({ id: current.shell } as NodeHandle);
    }
  }

  function paint(t: number, leaving: boolean): void {
    if (layer === null) {
      return;
    }
    const shift = leaving ? t : 1 - t;
    host.setAlpha(layer.fade, Math.round((leaving ? 1 - t : t) * 255));
    host.setTransform(layer.panel, 0, shift * 48, 0, 1);
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
    const scrimPaint = host.decorated(colorBytes(theme().color.scrim), 0);
    host.setFill(scrimPaint);
    host.insert(scrim, scrimPaint, 0);

    const column = host.column(0);
    host.setFill(column);
    host.setAlign(column, "end");
    host.insert(shell, column, 0);

    const fade = host.opacity(0);
    host.insert(column, fade, 0);
    const panel = host.transform();
    host.insert(fade, panel, 0);
    const surface = host.decorated(colorBytes(theme().color.surface), theme().radius.m);
    host.insert(panel, surface, 0);
    const inner = host.column(theme().space.m);
    host.insert(surface, inner, 0);

    const title = props.title ?? "";
    let titleId = 0;
    if (title.length > 0) {
      titleId = host.text(title, theme().fontSize.title, colorBytes(theme().color.text));
      host.setFontWeight(titleId, theme().weight.emphasis);
      host.setTextDirection(titleId, directionFor(theme().locale));
      host.insert(inner, titleId, 0);
    }
    host.setSemantics(shell, Role.Generic, title.length > 0 ? title : undefined);

    const slot = host.column(theme().space.s);
    host.insert(inner, slot, 0);
    layer = { shell, slot, panel, fade, titleId, previous: 0, back: 0, focused: false };
    return layer;
  }

  function armEnter(): void {
    stopSlide();
    const duration = theme().reduceMotion ? 0 : 220;
    stopSlide = transition((t) => paint(t, false), duration);
  }

  function beginClose(): void {
    if (layer === null || closing) {
      return;
    }
    closing = true;
    if (theme().reduceMotion) {
      closeLayer();
      return;
    }
    stopSlide();
    stopSlide = transition((t) => {
      paint(t, true);
      if (t >= 1) {
        closeLayer();
      }
    }, 220);
  }

  function sync(): void {
    if (placeholder === 0) {
      return;
    }
    if (!props.open) {
      beginClose();
      return;
    }
    const reopening = closing;
    if (closing) {
      stopSlide();
      closing = false;
    }
    const created = layer === null;
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
    if (created) {
      const previous = coreHost.focused();
      host.pushFocusScope(current.shell);
      current.previous = previous;
      current.back = host.onBack(() => {
        props.onClose();
        return true;
      });
      current.focused = true;
      takeInitialFocus(current.shell, props.initialFocus);
      armEnter();
    } else if (reopening) {
      armEnter();
    }
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
    stopSlide();
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
  panel: number;
  fade: number;
  titleId: number;
  previous: number;
  back: number;
  focused: boolean;
};
