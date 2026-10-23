import { colorBytes } from "./colors.ts";
import { BUTTON_FILL, FONT_SIZE, TEXT_COLOR } from "./defaults.ts";
import type { CoreHost } from "./host.ts";

/**
 * Renderer node. A bare number is text to `@solidjs/universal`, so the host
 * id stays on this object.
 */
export interface HostNode {
  readonly id: number;
}

/** Structural match for `@solidjs/universal` RendererOptions. Solid is not imported here. */
export interface HostRendererOptions {
  createElement(tag: string, staticProps?: Record<string, unknown>): HostNode;
  createTextNode(value: string | number): HostNode;
  replaceText(textNode: HostNode, value: string | number): void;
  isTextNode(node: HostNode): boolean;
  setProperty(node: HostNode, name: string, value: unknown, prev?: unknown): void;
  insertNode(parent: HostNode, node: HostNode, anchor?: HostNode): void;
  removeNode(parent: HostNode, node: HostNode): void;
  getParentNode(node: HostNode): HostNode | undefined;
  getFirstChild(node: HostNode): HostNode | undefined;
  getNextSibling(node: HostNode): HostNode | undefined;
}

function asNumber(value: unknown, fallback: number): number {
  return typeof value === "number" ? value : fallback;
}

function alignName(value: unknown): "start" | "center" | "end" {
  if (value === "center" || value === "end") {
    return value;
  }
  return "start";
}

function textAlignName(value: unknown): "left" | "center" | "right" {
  if (value === "center" || value === "right") {
    return value;
  }
  return "left";
}

function justifyName(
  value: unknown,
): "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly" {
  if (
    value === "center" ||
    value === "end" ||
    value === "space-between" ||
    value === "space-around" ||
    value === "space-evenly"
  ) {
    return value;
  }
  return "start";
}

type TransformValue = { x: number; y: number; rotate: number; scale: number };
type EdgeValue = { top: number; right: number; bottom: number; left: number };

function trackList(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.map((item) => String(item));
}

function sizePair(value: unknown): { w: number; h: number } {
  if (typeof value === "number") {
    return { w: value, h: value };
  }
  if (value !== null && typeof value === "object") {
    const record = value as { w?: unknown; h?: unknown };
    return { w: asNumber(record.w, 0), h: asNumber(record.h, 0) };
  }
  return { w: 0, h: 0 };
}

export function hostRendererOptions(host: CoreHost): HostRendererOptions {
  const nodes = new Map<number, HostNode>();
  const transforms = new Map<number, TransformValue>();
  const paddings = new Map<number, EdgeValue>();
  function writeTransform(id: number, patch: Partial<TransformValue>): void {
    const prev = transforms.get(id) ?? { x: 0, y: 0, rotate: 0, scale: 1 };
    const next = { ...prev, ...patch };
    transforms.set(id, next);
    host.setTransform(id, next.x, next.y, next.rotate, next.scale);
  }
  function writePadding(id: number, patch: Partial<EdgeValue>): void {
    const prev = paddings.get(id) ?? { top: 0, right: 0, bottom: 0, left: 0 };
    const next = { ...prev, ...patch };
    paddings.set(id, next);
    host.setPaddingEdges(id, next.top, next.right, next.bottom, next.left);
  }
  function wrap(id: number): HostNode {
    let node = nodes.get(id);
    if (node === undefined) {
      node = { id };
      nodes.set(id, node);
    }
    return node;
  }
  function defined(id: number): HostNode | undefined {
    return id === 0 ? undefined : wrap(id);
  }

  function setProperty(node: HostNode, name: string, value: unknown): void {
    switch (name) {
      case "gap":
        host.setGap(node.id, asNumber(value, 0));
        return;
      case "edges": {
        const edge = asNumber(value, 0);
        host.setPadding(node.id, edge);
        paddings.set(node.id, { top: edge, right: edge, bottom: edge, left: edge });
        return;
      }
      case "top":
        writePadding(node.id, { top: asNumber(value, 0) });
        return;
      case "right":
        writePadding(node.id, { right: asNumber(value, 0) });
        return;
      case "bottom":
        writePadding(node.id, { bottom: asNumber(value, 0) });
        return;
      case "left":
        writePadding(node.id, { left: asNumber(value, 0) });
        return;
      case "margin":
        if (typeof value === "number") {
          host.setMargin(node.id, value, value, value, value);
        }
        return;
      case "min": {
        const size = sizePair(value);
        host.setMinSize(node.id, size.w, size.h);
        return;
      }
      case "max": {
        const size = sizePair(value);
        host.setMaxSize(node.id, size.w, size.h);
        return;
      }
      case "shrink":
        host.setShrink(node.id, asNumber(value, 0));
        return;
      case "basis":
        host.setBasis(node.id, asNumber(value, 0));
        return;
      case "alignSelf":
        host.setAlignSelf(node.id, alignName(value));
        return;
      case "grow":
        host.setGrow(node.id, asNumber(value, 0));
        return;
      case "radius":
        host.setRadius(node.id, asNumber(value, 0));
        return;
      case "alpha":
        host.setAlpha(node.id, asNumber(value, 255));
        return;
      case "align":
        host.setAlign(node.id, alignName(value));
        return;
      case "maxLines":
        host.setMaxLines(node.id, asNumber(value, 0));
        return;
      case "ellipsis":
        host.setEllipsis(node.id, value == null ? "" : String(value));
        return;
      case "textAlign":
        host.setTextAlign(node.id, textAlignName(value));
        return;
      case "weight":
        host.setFontWeight(node.id, asNumber(value, 400));
        return;
      case "direction":
        host.setTextDirection(node.id, value === "rtl" ? "rtl" : "ltr");
        return;
      case "lineHeight":
        host.setLineHeight(node.id, asNumber(value, 0));
        return;
      case "family":
        host.setFamily(node.id, value == null ? "" : String(value));
        return;
      case "justify":
        host.setJustify(node.id, justifyName(value));
        return;
      case "wrap":
        host.setWrap(node.id, value === true);
        return;
      case "fill":
        if (value === true) {
          host.setFill(node.id);
        }
        return;
      case "focusable":
        host.setFocusable(node.id, value === true);
        return;
      case "x":
        writeTransform(node.id, { x: asNumber(value, 0) });
        return;
      case "y":
        writeTransform(node.id, { y: asNumber(value, 0) });
        return;
      case "rotate":
        writeTransform(node.id, { rotate: asNumber(value, 0) });
        return;
      case "scale":
        writeTransform(node.id, { scale: asNumber(value, 1) });
        return;
      case "fontSize":
        host.setFontSize(node.id, asNumber(value, FONT_SIZE));
        return;
      case "color":
        host.setColor(node.id, colorBytes(value));
        return;
      case "value":
        host.setText(node.id, value == null ? "" : String(value));
        return;
      case "onClick":
        if (typeof value === "function") {
          host.onClick(node.id, value as () => void);
        }
        return;
      case "onPress":
        if (typeof value === "function") {
          host.onPress(node.id, value as () => void);
        }
        return;
      case "onRelease":
        if (typeof value === "function") {
          host.onRelease(node.id, value as () => void);
        }
        return;
      case "onDragBegin":
        if (typeof value === "function") {
          host.onDragBegin(node.id, value as () => void);
        }
        return;
      case "onDragBy":
        if (typeof value === "function") {
          host.onDragBy(node.id, value as (dx: number, dy: number) => void);
        }
        return;
      case "onDragEnd":
        if (typeof value === "function") {
          host.onDragEnd(node.id, value as (vx: number, vy: number) => void);
        }
        return;
      case "onFocus":
        if (typeof value === "function") {
          host.onFocus(node.id, value as () => void);
        }
        return;
      case "onBlur":
        if (typeof value === "function") {
          host.onBlur(node.id, value as () => void);
        }
        return;
      case "onKey":
        if (typeof value === "function") {
          host.onKey(node.id, value as (key: string) => void);
        }
        return;
      case "onText":
        if (typeof value === "function") {
          host.onText(node.id, value as (text: string) => void);
        }
        return;
      case "onComposing":
        if (typeof value === "function") {
          host.onComposing(node.id, value as (text: string | null) => void);
        }
        return;
      case "onHover":
        if (typeof value === "function") {
          host.onHover(node.id, value as () => void);
        }
        return;
      case "onLeave":
        if (typeof value === "function") {
          host.onLeave(node.id, value as () => void);
        }
        return;
      case "onWheel":
        if (typeof value === "function") {
          host.onWheel(node.id, value as (dx: number, dy: number) => void);
        }
        return;
      case "onSecondaryClick":
        if (typeof value === "function") {
          host.onSecondaryClick(node.id, value as () => void);
        }
        return;
      case "children":
      case "ref":
        return;
      default:
        throw new Error(`unknown prop ${name}`);
    }
  }

  return {
    createElement(tag, staticProps) {
      let id: number;
      switch (tag) {
        case "column":
          id = host.column(asNumber(staticProps?.gap, 0));
          break;
        case "row":
          id = host.row(asNumber(staticProps?.gap, 0));
          break;
        case "stack":
          id = host.stack();
          break;
        case "padding": {
          const edge = asNumber(staticProps?.edges, 0);
          id = host.padding(edge);
          paddings.set(id, { top: edge, right: edge, bottom: edge, left: edge });
          break;
        }
        case "grid":
          id = host.grid(
            trackList(staticProps?.columns),
            trackList(staticProps?.rows),
            asNumber(staticProps?.gap, 0),
          );
          break;
        case "pointer":
          id = host.pointerRegion();
          break;
        case "clip":
          id = host.clip(asNumber(staticProps?.radius, 0));
          break;
        case "opacity":
          id = host.opacity(asNumber(staticProps?.alpha, 255));
          break;
        case "scroll":
          id = host.scroll();
          break;
        case "sized":
          id = host.sized(asNumber(staticProps?.w, 0), asNumber(staticProps?.h, 0));
          break;
        case "transform":
          id = host.transform();
          break;
        case "decorated":
          id = host.decorated(
            colorBytes(staticProps?.color ?? BUTTON_FILL),
            asNumber(staticProps?.radius, 0),
          );
          break;
        case "image":
          id = host.image(
            staticProps?.source == null ? "" : String(staticProps.source),
            asNumber(staticProps?.w, 0),
            asNumber(staticProps?.h, 0),
          );
          break;
        default:
          throw new Error(`unknown tag ${tag}`);
      }
      const node = wrap(id);
      if (staticProps !== undefined) {
        for (const [name, value] of Object.entries(staticProps)) {
          if (
            name === "gap" ||
            name === "edges" ||
            name === "columns" ||
            name === "rows" ||
            name === "color" ||
            name === "radius" ||
            name === "alpha" ||
            name === "source" ||
            name === "w" ||
            name === "h"
          ) {
            continue;
          }
          setProperty(node, name, value);
        }
      }
      return node;
    },
    createTextNode(value) {
      return wrap(host.text(String(value), FONT_SIZE, colorBytes(TEXT_COLOR)));
    },
    replaceText(textNode, value) {
      host.setText(textNode.id, String(value));
    },
    isTextNode(node) {
      return host.isText(node.id);
    },
    setProperty(node, name, value) {
      setProperty(node, name, value);
    },
    insertNode(parent, node, anchor) {
      host.insert(parent.id, node.id, anchor?.id ?? 0);
    },
    removeNode(parent, node) {
      host.remove(parent.id, node.id);
    },
    getParentNode(node) {
      return defined(host.parent(node.id));
    },
    getFirstChild(node) {
      return defined(host.firstChild(node.id));
    },
    getNextSibling(node) {
      return defined(host.nextSibling(node.id));
    },
  };
}
