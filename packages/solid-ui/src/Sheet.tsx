import { onCleanup } from "solid-js";
import {
  directionFor,
  scene,
  onFrame,
  overlayParent,
  removeNode,
  restoreFocus,
  spring,
  takeInitialFocus,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { sceneApi, nodeId } from "./scene-ext.ts";
import { registerOverlay } from "./overlay/registry.ts";
import { moveChildren, readFirstChild, restoreChildren } from "./overlay.ts";
import type { SheetProps } from "./props.ts";
import { Role } from "./semantics.ts";
import { createNodeRef, createStyleTarget } from "./target.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Bottom sheet with a scrim and a focus scope. Children park in a 0×0 clip.
 * Enter and leave slide; reduce-motion skips the in-between frames.
 */
export function Sheet(props: SheetProps): NodeHandle {
  const theme = useTheme();
  const panelStyle = createStyleTarget(() => props, () => ({
    backgroundColor: colorBytes(theme().color.surface), borderRadius: theme().radius.m,
  }));
  const scrimStyle = createStyleTarget(() => props.scrimProps, () => ({
    width: "100%", height: "100%", minWidth: 0, minHeight: 0, flex: 1, flexShrink: 1,
    backgroundColor: colorBytes(theme().color.scrim), borderRadius: 0,
  }));
  const contentStyle = createStyleTarget(() => props.contentProps, () => ({ gap: theme().space.s }));
  const titleStyle = createStyleTarget(() => props.titleProps, () => ({
    fontSize: theme().fontSize.title, color: colorBytes(theme().color.text),
    fontWeight: theme().weight.emphasis, writingDirection: directionFor(theme().locale),
  }));
  const panelRef = createNodeRef(() => props.ref);
  let placeholder = 0;
  let layer: Layer | null = null;
  let closing = false;
  let epoch = 0;
  function closeLayer(): void {
    if (layer === null) {
      return;
    }
    panelRef.clear();
    panelStyle.detach();
    scrimStyle.detach();
    contentStyle.detach();
    titleStyle.detach();
    const current = layer;
    layer = null;
    closing = false;
    try {
      restoreChildren(current.slot, placeholder);
      if (current.focused) {
        scene.popFocusScope();
        restoreFocus(current.previous);
      }
    } finally {
      if (current.unregister) {
        current.unregister();
      }
      removeNode(current.shell);
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
      write: (value) =>
        scene.patchStyle(fade, {
          opacity: Math.round(value) / 255,
        }),
      onDone: done,
    });
    spring({
      id: panel,
      prop: "translateY",
      from: leaving ? 0 : 48,
      to: leaving ? 48 : 0,
      write: (value) =>
        scene.patchStyle(panel, {
          translateX: 0,
          translateY: value,
          rotate: 0,
          scaleX: 1,
          scaleY: 1,
        }),
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
    const shell = scene.stack();
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
    const scrim = scene.createElement("view", {
      style: {},
    });
    scene.patchStyle(scrim, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.onClick(scrim, () => props.onClose());
    scene.insert(shell, scrim, 0);
    const scrimPaint = scene.createElement("view");
    scene.insert(scrim, scrimPaint, 0);
    const column = scene.createElement("view", {
      style: {
        gap: 0,
      },
    });
    scene.patchStyle(column, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.patchStyle(column, {
      justifyContent: "end",
    });
    scene.insert(shell, column, 0);
    const fade = scene.createElement("stack", {
      style: {
        opacity: 0 / 255,
      },
    });
    scene.insert(column, fade, 0);
    const panel = scene.createElement("stack", {
      style: {},
    });
    scene.insert(fade, panel, 0);
    const inner = scene.createElement("view", {
      style: {
        gap: theme().space.m,
      },
    });
    scene.insert(panel, inner, 0);
    const title = props.title ?? "";
    let titleId = 0;
    if (title.length > 0) {
      titleId = scene.createElement("text", { value: title });
      scene.insert(inner, titleId, 0);
    }
    scene.setSemantics(
      shell,
      Role.Generic,
      title.length > 0 ? title : undefined,
    );
    const slot = scene.createElement("view");
    scene.insert(inner, slot, 0);
    panelStyle.attach(panel);
    scrimStyle.attach(scrimPaint);
    contentStyle.attach(slot);
    panelRef.attach(panel);
    if (titleId !== 0) titleStyle.attach(titleId);
    layer = {
      shell,
      slot,
      panel,
      fade,
      titleId,
      previous: 0,
      focused: false,
      unregister: null,
    };
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
    const parked = readFirstChild(placeholder);
    if (parked === null) {
      placeholder = 0;
      closeLayer();
      return;
    }
    if (parked !== 0) {
      moveChildren(placeholder, current.slot);
    }
    if (created) {
      try {
        const previous = sceneApi.focused();
        scene.pushFocusScope(current.shell);
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
    <stack
      ref={(node) => {
        if (node === null) return;
        placeholder = node;
        scene.patchStyle(node, {
          minWidth: 0,
          minHeight: 0,
        });
        scene.patchStyle(node, {
          maxWidth: 0,
          maxHeight: 0,
        });
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
