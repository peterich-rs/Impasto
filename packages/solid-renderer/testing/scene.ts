import { colorBytes } from "../src/colors.ts";
import { BUTTON_FILL, FONT_SIZE, TEXT_COLOR } from "../src/defaults.ts";
import type { RecognizerSpec } from "../src/guard.ts";
import type {
  AxisSize,
  SceneModule,
  GridLine,
  TextSpanData,
  InsetEdge,
} from "../src/scene.ts";
import {
  parseAxisSize,
  parseGridLine,
  parseInsetEdge,
} from "../src/layout-values.ts";

/**
 * Renderer node. A bare number is text to `@solidjs/universal`, so the host
 * id stays on this object.
 */
import type { Node } from "../src/tree.ts";

/** Structural match for `@solidjs/universal` RendererOptions. Solid is not imported here. */
export interface SceneOptions {
  styles: Pick<
    SceneModule,
    "createStyleSource" | "setStyleSource" | "removeStyleSource"
  >;
  createElement(tag: string, staticProps?: Record<string, unknown>): Node;
  createTextNode(value: string | number): Node;
  replaceText(textNode: Node, value: string | number): void;
  isTextNode(node: Node): boolean;
  setProperty(node: Node, name: string, value: unknown, prev?: unknown): void;
  insertNode(parent: Node, node: Node, anchor?: Node): void;
  removeNode(parent: Node, node: Node): void;
  getParentNode(node: Node): Node | undefined;
  getFirstChild(node: Node): Node | undefined;
  getNextSibling(node: Node): Node | undefined;
}

function asNumber(value: unknown, fallback: number): number {
  return typeof value === "number" ? value : fallback;
}

/** `0` both, `1` vertical, `2` horizontal. */
function axisCode(value: unknown): number {
  if (value === "vertical") {
    return 1;
  }
  if (value === "horizontal") {
    return 2;
  }
  return 0;
}

function alignName(value: unknown): "start" | "center" | "end" {
  if (value === "center" || value === "end") {
    return value;
  }
  return "start";
}

function crossName(
  value: unknown,
): "start" | "center" | "end" | "stretch" | "baseline" {
  if (
    value === "start" ||
    value === "center" ||
    value === "end" ||
    value === "baseline"
  ) {
    return value;
  }
  return "stretch";
}

function overflowName(value: unknown): "clip" | "ellipsis" | "visible" {
  if (value === "ellipsis" || value === "visible") {
    return value;
  }
  return "clip";
}

function textAlignName(value: unknown): "left" | "center" | "right" {
  if (value === "center" || value === "right") {
    return value;
  }
  return "left";
}

function runColor(value: unknown): [number, number, number, number] {
  if (Array.isArray(value) && value.length === 4) {
    return [value[0] | 0, value[1] | 0, value[2] | 0, value[3] | 0];
  }
  return colorBytes(value ?? "#000000");
}

function normalizeRuns(value: unknown): TextSpanData[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.map((item) => {
    const run = item as {
      text?: unknown;
      color?: unknown;
      weight?: number;
      family?: string;
      underline?: boolean;
      lineThrough?: boolean;
      gradient?: {
        kind?: "linear" | "radial" | "sweep";
        from?: [number, number];
        to?: [number, number];
        center?: [number, number];
        radius?: number;
        startAngle?: number;
        endAngle?: number;
        stops?: { offset: number; color: unknown }[];
      };
    };
    const gradient = run.gradient;
    return {
      text: run.text == null ? "" : String(run.text),
      color: runColor(run.color),
      weight: run.weight,
      family: run.family,
      underline: run.underline,
      lineThrough: run.lineThrough,
      gradient: gradient
        ? {
            kind: gradient.kind ?? "linear",
            from: gradient.from,
            to: gradient.to,
            center: gradient.center,
            radius: gradient.radius,
            startAngle: gradient.startAngle,
            endAngle: gradient.endAngle,
            stops: (gradient.stops ?? []).map((stop) => ({
              offset: stop.offset,
              color: runColor(stop.color),
            })),
          }
        : undefined,
    };
  });
}

function justifyName(
  value: unknown,
):
  | "start"
  | "center"
  | "end"
  | "space-between"
  | "space-around"
  | "space-evenly" {
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

export type RetainedCounts = {
  nodes: number;
};

// Test double for Node component tests; production imports only the native scene module.
export function testScene(host: SceneModule): SceneOptions & {
  retainedCounts(): RetainedCounts;
  forget(id: number): void;
} {
  const nodes = new Map<number, Node>();

  function forget(id: number, seen = new Set<number>()): void {
    if (id === 0 || seen.has(id)) {
      return;
    }
    seen.add(id);
    const children: number[] = [];
    const listed = new Set<number>();
    let child = host.firstChild(id);
    while (child !== 0 && !seen.has(child) && !listed.has(child)) {
      children.push(child);
      listed.add(child);
      child = host.nextSibling(child);
    }
    for (const kid of children) {
      forget(kid, seen);
    }
    nodes.delete(id);
  }
  function insetValue(value: InsetEdge): number | null {
    return value === "auto" ? null : value;
  }
  function sizeCode(value: AxisSize): { kind: number; value: number } {
    if (value === "auto") {
      return { kind: 0, value: 0 };
    }
    if (typeof value === "number") {
      return { kind: 1, value };
    }
    return { kind: 2, value: Number(value.slice(0, -1)) / 100 };
  }
  function lineCode(value: GridLine): { kind: number; value: number } {
    if (value === "auto") {
      return { kind: 0, value: 0 };
    }
    if ("line" in value) {
      return { kind: 1, value: value.line };
    }
    return { kind: 2, value: value.span };
  }
  function wrap(id: number): Node {
    let node = nodes.get(id);
    if (node === undefined) {
      node = id;
      nodes.set(id, node);
    }
    return node;
  }
  function defined(id: number): Node | undefined {
    return id === 0 ? undefined : wrap(id);
  }

  function setProperty(node: Node, name: string, value: unknown): void {
    switch (name) {
      case "style":
        host.setStyle(node, value as import("../src/style.ts").StyleInput);
        return;
      case "class":
        host.setClass(node, String(value ?? ""));
        return;
      case "classList":
        host.setClass(
          node,
          Object.entries((value as Record<string, boolean>) ?? {})
            .filter(([, on]) => on)
            .map(([name]) => name)
            .join(" "),
        );
        return;
      case "gap":
        host.setGap(node, asNumber(value, 0));
        return;
      case "edges":
        host.setPadding(node, asNumber(value, 0));
        return;
      case "top":
        host.setPaddingEdge(node, 0, asNumber(value, 0));
        return;
      case "right":
        host.setPaddingEdge(node, 1, asNumber(value, 0));
        return;
      case "bottom":
        host.setPaddingEdge(node, 2, asNumber(value, 0));
        return;
      case "left":
        host.setPaddingEdge(node, 3, asNumber(value, 0));
        return;
      case "margin":
        if (typeof value === "number") {
          host.setMargin(node, value, value, value, value);
        }
        return;
      case "min": {
        const size = sizePair(value);
        host.setMinSize(node, size.w, size.h);
        return;
      }
      case "max": {
        const size = sizePair(value);
        host.setMaxSize(node, size.w, size.h);
        return;
      }
      case "shrink":
        host.setShrink(node, asNumber(value, 0));
        return;
      case "basis":
        host.setBasis(node, asNumber(value, 0));
        return;
      case "alignSelf":
        host.setAlignSelf(node, alignName(value));
        return;
      case "grow":
        host.setGrow(node, asNumber(value, 0));
        return;
      case "radius":
        host.setRadius(node, asNumber(value, 0));
        return;
      case "alpha":
        host.setAlpha(node, asNumber(value, 255));
        return;
      case "align":
        host.setAlign(node, alignName(value));
        return;
      case "maxLines":
        host.setMaxLines(node, asNumber(value, 0));
        return;
      case "ellipsis":
        host.setEllipsis(node, value == null ? "" : String(value));
        return;
      case "softWrap":
        host.setSoftWrap(node, value !== false);
        return;
      case "overflow":
        host.setOverflow(node, overflowName(value));
        return;
      case "textAlign":
        host.setTextAlign(node, textAlignName(value));
        return;
      case "weight":
        host.setFontWeight(node, asNumber(value, 400));
        return;
      case "direction":
        host.setTextDirection(node, value === "rtl" ? "rtl" : "ltr");
        return;
      case "lineHeight":
        host.setLineHeight(node, asNumber(value, 0));
        return;
      case "w": {
        const size = sizeCode(parseAxisSize(value));
        host.setSizeAxis(node, 0, size.kind, size.value);
        return;
      }
      case "h": {
        const size = sizeCode(parseAxisSize(value));
        host.setSizeAxis(node, 1, size.kind, size.value);
        return;
      }
      case "aspect":
        if (value === null) {
          host.setAspectRatio(node, null);
          return;
        }
        if (typeof value === "number" && Number.isFinite(value) && value > 0) {
          host.setAspectRatio(node, value);
          return;
        }
        throw new Error("bad aspect");
      case "absolute":
        host.setPosition(node, value === true);
        return;
      case "insetTop":
        host.setInsetEdge(node, 0, insetValue(parseInsetEdge(value)));
        return;
      case "insetRight":
        host.setInsetEdge(node, 1, insetValue(parseInsetEdge(value)));
        return;
      case "insetBottom":
        host.setInsetEdge(node, 2, insetValue(parseInsetEdge(value)));
        return;
      case "insetLeft":
        host.setInsetEdge(node, 3, insetValue(parseInsetEdge(value)));
        return;
      case "columnStart": {
        const line = lineCode(parseGridLine(value));
        host.setGridLine(node, 0, line.kind, line.value);
        return;
      }
      case "columnEnd": {
        const line = lineCode(parseGridLine(value));
        host.setGridLine(node, 1, line.kind, line.value);
        return;
      }
      case "rowStart": {
        const line = lineCode(parseGridLine(value));
        host.setGridLine(node, 2, line.kind, line.value);
        return;
      }
      case "rowEnd": {
        const line = lineCode(parseGridLine(value));
        host.setGridLine(node, 3, line.kind, line.value);
        return;
      }
      case "family":
        host.setFamily(node, value == null ? "" : String(value));
        return;
      case "justify":
        host.setJustify(node, justifyName(value));
        return;
      case "cross":
        host.setCross(node, crossName(value));
        return;
      case "wrap":
        host.setWrap(node, value === true);
        return;
      case "fill":
        if (value === true) {
          host.setFill(node);
        }
        return;
      case "axis":
        host.setScrollAxis(node, axisCode(value));
        return;
      case "focusable":
        host.setFocusable(node, value === true);
        return;
      case "x":
        host.setTransformComponent(node, 0, asNumber(value, 0));
        return;
      case "y":
        host.setTransformComponent(node, 1, asNumber(value, 0));
        return;
      case "rotate":
        host.setTransformComponent(node, 2, asNumber(value, 0));
        return;
      case "scale":
        host.setTransformComponent(node, 3, asNumber(value, 1));
        return;
      case "fontSize":
        host.setFontSize(node, asNumber(value, FONT_SIZE));
        return;
      case "color":
        host.setColor(node, colorBytes(value));
        return;
      case "runs":
        host.setTextRuns(node, normalizeRuns(value));
        return;
      case "value":
        host.setText(node, value == null ? "" : String(value));
        return;
      case "onClick":
        if (typeof value === "function") {
          host.onClick(node, value as () => void);
        }
        return;
      case "onPress":
        if (typeof value === "function") {
          host.onPress(node, value as () => void);
        }
        return;
      case "onRelease":
        if (typeof value === "function") {
          host.onRelease(node, value as () => void);
        }
        return;
      case "onDragBegin":
        if (typeof value === "function") {
          host.onDragBegin(node, value as () => void);
        }
        return;
      case "onDragBy":
        if (typeof value === "function") {
          host.onDragBy(node, value as (dx: number, dy: number) => void);
        }
        return;
      case "onDragEnd":
        if (typeof value === "function") {
          host.onDragEnd(node, value as (vx: number, vy: number) => void);
        }
        return;
      case "onFocus":
        if (typeof value === "function") {
          host.onFocus(node, value as () => void);
        }
        return;
      case "onBlur":
        if (typeof value === "function") {
          host.onBlur(node, value as () => void);
        }
        return;
      case "onKey":
        if (typeof value === "function") {
          host.onKey(node, value as (key: string) => void);
        }
        return;
      case "onText":
        if (typeof value === "function") {
          host.onText(node, value as (text: string) => void);
        }
        return;
      case "onTextReplaced":
        if (typeof value === "function") {
          host.onTextReplaced(node, value as (text: string) => void);
        }
        return;
      case "onLink":
        if (typeof value === "function") {
          host.onLink(node, value as (link: number) => void);
        }
        return;
      case "onComposing":
        if (typeof value === "function") {
          host.onComposing(node, value as (text: string | null) => void);
        }
        return;
      case "onHover":
        if (typeof value === "function") {
          host.onHover(node, value as () => void);
        }
        return;
      case "onLeave":
        if (typeof value === "function") {
          host.onLeave(node, value as () => void);
        }
        return;
      case "onWheel":
        if (typeof value === "function") {
          host.onWheel(node, value as (dx: number, dy: number) => void);
        }
        return;
      case "onScroll":
        if (typeof value === "function") {
          host.onScroll(node, value as (x: number, y: number) => void);
        }
        return;
      case "onSecondaryClick":
        if (typeof value === "function") {
          host.onSecondaryClick(node, value as () => void);
        }
        return;
      case "onZoom":
        if (typeof value === "function" && typeof host.onZoom === "function") {
          host.onZoom(node, value as (scale: number) => void);
        }
        return;
      case "onLongPress":
        if (typeof value === "function") {
          host.onLongPress(node, value as () => void);
        }
        return;
      case "recognizers":
        if (
          value &&
          typeof value === "object" &&
          typeof host.setRecognizers === "function"
        ) {
          host.setRecognizers(node, value as RecognizerSpec);
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
    styles: host,
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
        case "align":
          id = host.align(alignName(staticProps?.align));
          break;
        case "padding": {
          const edge = asNumber(staticProps?.edges, 0);
          id = host.padding(edge);
          break;
        }
        case "grid":
          id = host.grid(
            trackList(staticProps?.columns),
            trackList(staticProps?.rows),
            asNumber(staticProps?.gap, 0),
          );
          break;
        case "text":
          id = host.text(
            String(staticProps?.value ?? ""),
            14,
            [24, 24, 27, 255],
          );
          break;
        case "view":
          id = host.view();
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
          id = host.scroll(axisCode(staticProps?.axis));
          break;
        case "sized": {
          const width =
            staticProps?.w === undefined ? 0 : parseAxisSize(staticProps.w);
          const height =
            staticProps?.h === undefined ? 0 : parseAxisSize(staticProps.h);
          id = host.sized(
            typeof width === "number" ? width : 0,
            typeof height === "number" ? height : 0,
          );
          if (typeof width !== "number" || typeof height !== "number") {
            host.setAxisSize(id, width, height);
          }
          break;
        }
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
            staticProps?.mode == null ? "fill" : String(staticProps.mode),
            staticProps?.required === true,
          );
          break;
        default:
          throw new Error(`unknown tag ${tag}`);
      }
      const node = wrap(id);
      if (staticProps !== undefined) {
        for (const [name, value] of Object.entries(staticProps)) {
          if (
            name === "value" ||
            name === "gap" ||
            name === "edges" ||
            name === "columns" ||
            name === "rows" ||
            name === "color" ||
            name === "radius" ||
            name === "alpha" ||
            name === "source" ||
            name === "w" ||
            name === "h" ||
            name === "mode" ||
            name === "required" ||
            name === "axis"
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
      host.setText(textNode, String(value));
    },
    isTextNode(node) {
      return host.isText(node);
    },
    setProperty(node, name, value) {
      setProperty(node, name, value);
    },
    insertNode(parent, node, anchor) {
      host.insert(parent, node, anchor ?? 0);
    },
    removeNode(parent, node) {
      forget(node);
      host.remove(parent, node);
    },
    getParentNode(node) {
      return defined(host.parent(node));
    },
    getFirstChild(node) {
      return defined(host.firstChild(node));
    },
    getNextSibling(node) {
      return defined(host.nextSibling(node));
    },
    retainedCounts() {
      return { nodes: nodes.size };
    },
    forget(id: number) {
      forget(id);
    },
  };
}
