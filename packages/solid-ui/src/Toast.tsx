import { onCleanup } from "solid-js";
import {
  directionFor,
  scene,
  onFrame,
  overlayParent,
  removeNode,
  requestFrame,
  spring,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./scene-ext.ts";
import type { ToastProps } from "./props.ts";
import { Role } from "./semantics.ts";
import { createNodeRef, createStyleTarget } from "./target.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal message. No scrim and no focus scope. Closes after `durationMs`.
 * Leave runs in reverse and removes the shell only after it finishes.
 */
export function Toast(props: ToastProps): NodeHandle {
  const theme = useTheme();
  const panelStyle = createStyleTarget(() => props, () => ({
    backgroundColor: colorBytes(theme().color.surface), borderRadius: theme().radius.m,
  }));
  const textStyle = createStyleTarget(() => props.textProps, () => ({
    fontSize: theme().fontSize.body, color: colorBytes(theme().color.text),
    fontWeight: theme().weight.body, writingDirection: directionFor(theme().locale),
  }));
  const panelRef = createNodeRef(() => props.ref);
  let shell = 0;
  let panel = 0;
  let fade = 0;
  let label = 0;
  let openedAt: number | null = null;
  let closing = false;
  function slide(leaving: boolean, onDone?: () => void): void {
    if (fade === 0 || panel === 0) {
      onDone?.();
      return;
    }
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
      from: leaving ? 0 : 24,
      to: leaving ? 24 : 0,
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
  function dismiss(): void {
    panelRef.clear();
    panelStyle.detach();
    textStyle.detach();
    if (shell !== 0) {
      removeNode(shell);
    }
    shell = 0;
    panel = 0;
    fade = 0;
    label = 0;
    openedAt = null;
    closing = false;
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
    const frame = scene.createElement("view", {
      style: {
        alignItems: "end",
        justifyContent: "end",
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
    const pad = scene.createElement("view", {
      style: {
        padding: theme().space.l,
      },
    });
    scene.insert(frame, pad, 0);
    fade = scene.createElement("stack", {
      style: {
        opacity: 0 / 255,
      },
    });
    scene.insert(pad, fade, 0);
    panel = scene.createElement("stack", {
      style: {},
    });
    scene.insert(fade, panel, 0);
    const inner = scene.createElement("view", {
      style: {
        padding: theme().space.m,
      },
    });
    scene.insert(panel, inner, 0);
    label = scene.createElement("text", { value: props.text });
    scene.insert(inner, label, 0);
    panelStyle.attach(panel);
    textStyle.attach(label);
    panelRef.attach(panel);
    scene.setSemantics(
      shell,
      Role.Text,
      props.text,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      {
        live: "polite",
      },
    );
  }
  function beginClose(): void {
    if (shell === 0 || closing) {
      return;
    }
    closing = true;
    slide(true, () => dismiss());
  }
  function sync(targetMs: number): void {
    if (props.open) {
      if (closing) {
        closing = false;
      }
      const born = shell === 0;
      ensure();
      if (label !== 0) {
        scene.setText(label, props.text);
      }
      if (born) {
        openedAt = targetMs;
        slide(false);
      }
      const duration = props.durationMs ?? 3000;
      if (openedAt !== null && targetMs - openedAt >= duration) {
        props.onClose();
        requestFrame();
        return;
      }
      requestFrame();
      return;
    }
    beginClose();
  }
  const stop = onFrame((targetMs) => {
    sync(targetMs);
  });
  onCleanup(() => {
    stop();
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
