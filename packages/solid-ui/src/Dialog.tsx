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
import type { DialogProps } from "./props.ts";
import { Role } from "./semantics.ts";
import { createNodeRef, createStyleTarget } from "./target.ts";
import { useTheme } from "./use-theme.ts";

/**
 * A 0×0 placeholder. While open, children live in the overlay column.
 * Open and close fade through `scene.animate`. The shell stays until the spring settles.
 */
export function Dialog(props: DialogProps): NodeHandle {
  const theme = useTheme();
  const panelStyle = createStyleTarget(() => props, () => ({ gap: theme().space.m }));
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
      current.unregister?.();
      removeNode(current.fade);
    }
  }
  function beginClose(): void {
    if (layer === null || closing) {
      return;
    }
    closing = true;
    const fade = layer.fade;
    const token = ++epoch;
    spring({
      id: fade,
      prop: "opacity",
      from: 255,
      to: 0,
      write: (value) =>
        scene.patchStyle(fade, {
          opacity: Math.round(value) / 255,
        }),
      onDone: () => {
        if (token === epoch) {
          closeLayer();
          props.onClosed?.();
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
    const fade = scene.createElement("stack", {
      style: {
        opacity: 0 / 255,
      },
    });
    scene.patchStyle(fade, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.insert(overlay, fade, 0);
    const shell = scene.createElement("stack");
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
    scene.insert(fade, shell, 0);
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
    const paint = scene.createElement("view");
    scene.insert(scrim, paint, 0);
    const frame = scene.createElement("view", {
      style: {
        alignItems: "center",
        justifyContent: "center",
      },
    });
    scene.patchStyle(frame, {
      width: "100%",
      height: "100%",
      minWidth: 0,
      minHeight: 0,
      flex: 1,
      flexShrink: 1,
    });
    scene.insert(shell, frame, 0);
    const content = scene.createElement("view");
    scene.insert(frame, content, 0);
    const title = props.title ?? "";
    let titleId = 0;
    if (title.length > 0) {
      titleId = scene.createElement("text", { value: title });
      scene.insert(content, titleId, 0);
    }
    scene.setSemantics(
      shell,
      Role.Generic,
      title.length > 0 ? title : undefined,
    );
    const slot = scene.createElement("view");
    scene.insert(content, slot, 0);
    panelStyle.attach(content);
    scrimStyle.attach(paint);
    contentStyle.attach(slot);
    panelRef.attach(content);
    if (titleId !== 0) titleStyle.attach(titleId);
    layer = {
      shell,
      slot,
      titleId,
      fade,
      previous: 0,
      back: 0,
      focused: false,
      unregister: null,
    };
    return layer;
  }
  function armEnter(current: Layer): void {
    spring({
      id: current.fade,
      prop: "opacity",
      from: 0,
      to: 255,
      write: (value) =>
        scene.patchStyle(current.fade, {
          opacity: Math.round(value) / 255,
        }),
    });
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
    if (reopening) {
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
    if (created || reopening) armEnter(current);
    if (current.focused) {
      return;
    }
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
  titleId: number;
  fade: number;
  previous: number;
  back: number;
  focused: boolean;
  unregister: (() => void) | null;
};
