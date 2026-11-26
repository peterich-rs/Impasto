import { createRenderEffect } from "solid-js";
import {
  bindNodeRef,
  bindStyle,
  scene,
  type NodeHandle,
  type StyleInput,
  type ViewEvents,
} from "@impasto/solid-renderer";
import type { ViewProps } from "./props.ts";
import { Role } from "./semantics.ts";

export function eventsOf(props: ViewEvents): ViewEvents {
  return {
    onClick: props.onClick,
    onPress: props.onPress,
    onRelease: props.onRelease,
    onFocus: props.onFocus,
    onBlur: props.onBlur,
    onKey: props.onKey,
    onText: props.onText,
    onTextReplaced: props.onTextReplaced,
    onLink: props.onLink,
    onComposing: props.onComposing,
    onDragBegin: props.onDragBegin,
    onDragBy: props.onDragBy,
    onDragEnd: props.onDragEnd,
    onHover: props.onHover,
    onLeave: props.onLeave,
    onSecondaryClick: props.onSecondaryClick,
    onZoom: props.onZoom,
    onLongPress: props.onLongPress,
    onWheel: props.onWheel,
    recognizers: props.recognizers,
    focusable: props.focusable,
  };
}

type PressEvents = Required<Pick<ViewEvents,
  "onPress" | "onRelease" | "onClick" | "onFocus" | "onBlur" | "onKey" | "onText"
>>;

const forwardedPressEvents = [
  "onTextReplaced", "onLink", "onComposing", "onDragBegin", "onDragBy", "onDragEnd",
  "onHover", "onLeave", "onSecondaryClick", "onZoom", "onLongPress", "onWheel", "recognizers",
] as const satisfies readonly (keyof ViewEvents)[];

export function bindViewEvents(node: NodeHandle, props: ViewEvents, press: PressEvents): void {
  scene.onPress(node, press.onPress);
  scene.onRelease(node, press.onRelease);
  scene.onClick(node, press.onClick);
  scene.onFocus(node, press.onFocus);
  scene.onBlur(node, press.onBlur);
  scene.onKey(node, press.onKey);
  scene.onText(node, press.onText);
  let previous: unknown[] = [];
  createRenderEffect(
    () => forwardedPressEvents.map(name => props[name]),
    (next) => {
      for (let index = 0; index < next.length; index++) {
        if (next[index] !== previous[index]) {
          scene.setProp(node, forwardedPressEvents[index], next[index]);
        }
      }
      previous = next;
    },
  );
}

export function bindBox(
  node: NodeHandle | null,
  props: ViewProps,
  defaults?: StyleInput | (() => StyleInput),
  disabled?: () => boolean,
): void {
  if (node === null) return;
  bindStyle(node, {
    get defaults() {
      return typeof defaults === "function" ? defaults() : defaults;
    },
    get class() {
      return props.class;
    },
    get classList() {
      return props.classList;
    },
    get style() {
      return props.style;
    },
  });
  bindNodeRef(node, () => props.ref);
  let installed = false;
  createRenderEffect(
    () => ({
      role: props.role,
      label: props.accessibilityLabel,
      hint: props.accessibilityHint,
      focusable: props.focusable,
      off: disabled?.(),
    }),
    ({ role, label, hint, focusable, off }) => {
      const labeled = role !== undefined || label !== undefined || hint !== undefined;
      if (off !== undefined) scene.setFocusable(node, !off);
      else if (focusable !== undefined) scene.setFocusable(node, focusable);
      if (!labeled && !installed && off === undefined) return;
      installed = labeled || off !== undefined;
      scene.setSemantics(
        node, role ?? Role.Generic, label,
        undefined, undefined, undefined, undefined,
        off === undefined ? undefined : off ? 1 : 0,
        undefined, undefined, hint,
      );
    },
  );
}
