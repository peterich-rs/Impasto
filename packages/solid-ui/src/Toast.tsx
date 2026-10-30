import { onCleanup } from "solid-js";
import { directionFor, host, onFrame, overlayParent, removeNode, requestFrame, spring, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "./colors.ts";
import { nodeId } from "./host-ext.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

/**
 * Non-modal message. No scrim and no focus scope. Closes after `durationMs`.
 * Leave runs in reverse and removes the shell only after it finishes.
 */
export function Toast(props: {
  open: boolean;
  text: string;
  durationMs?: number;
  onClose: () => void;
}): NodeHandle {
  const theme = useTheme();
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
      write: (value) => host.setAlpha(fade, Math.round(value)),
      onDone: done,
    });
    spring({
      id: panel,
      prop: "translateY",
      from: leaving ? 0 : 24,
      to: leaving ? 24 : 0,
      write: (value) => host.setTransform(panel, 0, value, 0, 1),
      onDone: done,
    });
  }

  function dismiss(): void {
    if (shell !== 0) {
      removeNode({ id: shell } as NodeHandle);
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
    shell = host.stack();
    host.setFill(shell);
    host.insert(overlay, shell, 0);
    const frame = host.align("end");
    host.setFill(frame);
    host.insert(shell, frame, 0);
    const pad = host.padding(theme().space.l);
    host.insert(frame, pad, 0);
    fade = host.opacity(0);
    host.insert(pad, fade, 0);
    panel = host.transform();
    host.insert(fade, panel, 0);
    const paintNode = host.decorated(colorBytes(theme().color.surface), theme().radius.m);
    host.insert(panel, paintNode, 0);
    const inner = host.padding(theme().space.m);
    host.insert(paintNode, inner, 0);
    label = host.text(props.text, theme().fontSize.body, colorBytes(theme().color.text));
    host.setFontWeight(label, theme().weight.body);
    host.setTextDirection(label, directionFor(theme().locale));
    host.insert(inner, label, 0);
    host.setSemantics(shell, Role.Text, props.text, undefined, undefined, undefined, undefined, undefined, undefined, undefined, undefined, {
      live: "polite",
    });
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
        host.setText(label, props.text);
        host.setTextDirection(label, directionFor(theme().locale));
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

  return <clip min={{ w: 0, h: 0 }} max={{ w: 0, h: 0 }} />;
}
