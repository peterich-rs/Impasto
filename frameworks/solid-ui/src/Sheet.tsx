import { onCleanup } from "solid-js";
import { directionFor, host, onFrame, overlayParent, removeNode, restoreFocus, spring, takeInitialFocus, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { coreHost, nodeId } from "./host-ext.ts";
import { registerOverlay } from "./overlay/registry.ts";
import { moveChildren } from "./overlay.ts";
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
      if (current.unregister) {
        current.unregister();
      }
      removeNode({ id: current.shell } as NodeHandle);
    }
  }

  function play(leaving: boolean, onDone?: () => void): void {
    if (layer === null) {
      return;
    }
    const fade = layer.fade;
    const panel = layer.panel;
    let left = 2;
    const done = () => {
      left -= 1;
      if (left <= 0) {
        onDone?.();
      }
    };
    spring({
      id: fade,
      prop: "opacity",
      from: leaving ? 255 : 0,
      to: leaving ? 0 : 255,
      write: (value) => host.setAlpha(fade, Math.round(value)),
      onDone: done,
    });
    spring({
      id: panel,
      prop: "translateY",
      from: leaving ? 0 : 48,
      to: leaving ? 48 : 0,
      write: (value) => host.setTransform(panel, 0, value, 0, 1),
      onDone: done,
    });
  }

  function armEnter(): void {
    play(false);
  }

  function beginClose(): void {
    if (layer === null || closing) {
      return;
    }
    closing = true;
    layer.unregister?.();
    layer.unregister = null;
    const token = ++epoch;
    play(true, () => {
      if (token === epoch) {
        closeLayer();
      }
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
    host.setJustify(column, "end");
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
    layer = { shell, slot, panel, fade, titleId, previous: 0, focused: false, unregister: null };
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
    const reopening = closing;
    if (closing) {
      epoch += 1;
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
      armEnter();
    } else if (reopening) {
      armEnter();
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
  panel: number;
  fade: number;
  titleId: number;
  previous: number;
  focused: boolean;
  unregister: (() => void) | null;
};
