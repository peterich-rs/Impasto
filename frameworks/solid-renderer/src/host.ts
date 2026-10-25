/** Stable scene-node handle. 0 means none, at the QuickJS boundary only. */
export type NodeHandle = number;

export type { AxisSize, GridLine, InsetEdge } from "./layout-values.ts";
import type { AxisSize, GridLine, InsetEdge } from "./layout-values.ts";

/** Platform settings projected into the script. Field names match the session. */
export type PlatformSnapshot = {
  locale: string;
  textScale: number;
  brightness: "light" | "dark";
  reduceMotion: boolean;
  boldText: boolean;
  highContrast: boolean;
};

/**
 * The `core` object QuickJS installs. Method names match `glue.c`.
 * Handles are JS numbers. A live id is never 0.
 */
export interface CoreHost {
  column(gap: number): NodeHandle;
  row(gap: number): NodeHandle;
  stack(): NodeHandle;
  padding(edges: number): NodeHandle;
  grid(columns: string[], rows: string[], gap: number): NodeHandle;
  pointerRegion(): NodeHandle;
  decorated(color: number[], radius: number): NodeHandle;
  clip(radius: number): NodeHandle;
  opacity(alpha: number): NodeHandle;
  scroll(): NodeHandle;
  transform(): NodeHandle;
  image(source: string, width: number, height: number, mode?: string, required?: boolean): NodeHandle;
  setImageMode(node: NodeHandle, mode: string, required: boolean): void;
  text(utf8: string, fontSize: number, color: number[]): NodeHandle;
  append(parent: NodeHandle, child: NodeHandle): void;
  insert(parent: NodeHandle, child: NodeHandle, before: NodeHandle): void;
  remove(parent: NodeHandle, child: NodeHandle): void;
  parent(node: NodeHandle): NodeHandle;
  firstChild(node: NodeHandle): NodeHandle;
  nextSibling(node: NodeHandle): NodeHandle;
  isText(node: NodeHandle): boolean;
  setText(node: NodeHandle, utf8: string): void;
  setFontSize(node: NodeHandle, fontSize: number): void;
  setColor(node: NodeHandle, color: number[]): void;
  setGap(node: NodeHandle, gap: number): void;
  setPadding(node: NodeHandle, edges: number): void;
  setPaddingEdges(node: NodeHandle, top: number, right: number, bottom: number, left: number): void;
  setMargin(node: NodeHandle, top: number, right: number, bottom: number, left: number): void;
  setMinSize(node: NodeHandle, width: number, height: number): void;
  setMaxSize(node: NodeHandle, width: number, height: number): void;
  setShrink(node: NodeHandle, shrink: number): void;
  setBasis(node: NodeHandle, basis: number): void;
  setAlignSelf(node: NodeHandle, align: "start" | "center" | "end"): void;
  setGrow(node: NodeHandle, grow: number): void;
  setRadius(node: NodeHandle, radius: number): void;
  setAlpha(node: NodeHandle, alpha: number): void;
  setAlign(node: NodeHandle, align: "start" | "center" | "end"): void;
  setSize(node: NodeHandle, width: number, height: number): void;
  setImage(node: NodeHandle, source: string, width: number, height: number): void;
  setMaxLines(node: NodeHandle, lines: number): void;
  setEllipsis(node: NodeHandle, text: string): void;
  setSoftWrap(node: NodeHandle, wrap: boolean): void;
  setOverflow(node: NodeHandle, overflow: "clip" | "ellipsis" | "visible"): void;
  setTextAlign(node: NodeHandle, align: "left" | "center" | "right"): void;
  setFontWeight(node: NodeHandle, weight: number): void;
  setTextDirection(node: NodeHandle, direction: "ltr" | "rtl"): void;
  setLineHeight(node: NodeHandle, height: number): void;
  setFamily(node: NodeHandle, family: string): void;
  setScrollOffset(node: NodeHandle, x: number, y: number): void;
  scrollOffset(node: NodeHandle): { x: number; y: number };
  rect(node: NodeHandle): { x: number; y: number; w: number; h: number };
  setTransform(node: NodeHandle, x: number, y: number, rotate: number, scale: number): void;
  setJustify(
    node: NodeHandle,
    justify: "start" | "center" | "end" | "space-between" | "space-around" | "space-evenly",
  ): void;
  setWrap(node: NodeHandle, wrap: boolean): void;
  setFill(node: NodeHandle): void;
  sized(width: number, height: number): NodeHandle;
  setFocusable(node: NodeHandle, enabled: boolean): void;
  focus(node: NodeHandle): void;
  /** Clears focus. No argument: `Tree::focus(0)` is an unknown node. */
  blur(): void;
  /** `0` means nothing is focused. */
  focused(): number;
  isFocusable(node: NodeHandle): boolean;
  setImeEnabled(enabled: boolean): void;
  setImeText(text: string): void;
  setImeState(
    text: string | null,
    selectionStart?: number,
    selectionEnd?: number,
    composingStart?: number,
    composingEnd?: number,
    x?: number,
    y?: number,
    w?: number,
    h?: number,
  ): void;
  setImeConfig(keyboard: number, action: number): void;
  setSemantics(
    node: NodeHandle,
    role: number,
    label?: string,
    value?: string,
    checked?: number,
    selectionStart?: number,
    selectionEnd?: number,
    disabled?: number,
    editable?: number,
    readOnly?: number,
    description?: string,
  ): void;
  textScale(): number;
  setTextScale(scale: number): void;
  reduceMotion(): boolean;
  settings(): PlatformSnapshot;
  onSettings(fn: (next: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
  graphemeIndex(text: string, utf16: number): number;
  graphemeCount(text: string): number;
  graphemeUnit(text: string, index: number): number;
  setReverse(node: NodeHandle, reverse: boolean): void;
  setScaleXY(node: NodeHandle, scaleX: number, scaleY: number): void;
  registerFont(family: string, bytes: Uint8Array): void;
  reload(source: string): void;
  setAxisSize(node: NodeHandle, width: AxisSize, height: AxisSize): void;
  setAspectRatio(node: NodeHandle, ratio: number | null): void;
  setPosition(node: NodeHandle, absolute: boolean): void;
  setInset(
    node: NodeHandle,
    top: InsetEdge,
    right: InsetEdge,
    bottom: InsetEdge,
    left: InsetEdge,
  ): void;
  setGridPlacement(
    node: NodeHandle,
    columnStart: GridLine,
    columnEnd: GridLine,
    rowStart: GridLine,
    rowEnd: GridLine,
  ): void;
  measureText(
    text: string,
    fontSize: number,
    maxWidth: number,
    color: number[],
  ): { w: number; h: number };
  requestFrame(): void;
  onClick(node: NodeHandle, fn: () => void): void;
  onPress(node: NodeHandle, fn: () => void): void;
  onRelease(node: NodeHandle, fn: () => void): void;
  onDragBegin(node: NodeHandle, fn: () => void): void;
  onDragBy(node: NodeHandle, fn: (dx: number, dy: number) => void): void;
  onDragEnd(node: NodeHandle, fn: (vx: number, vy: number) => void): void;
  onFocus(node: NodeHandle, fn: () => void): void;
  onBlur(node: NodeHandle, fn: () => void): void;
  onKey(node: NodeHandle, fn: (key: string) => void): void;
  onText(node: NodeHandle, fn: (text: string) => void): void;
  onTextReplaced(node: NodeHandle, fn: (text: string) => void): void;
  onLink(node: NodeHandle, fn: (link: number) => void): void;
  onComposing(node: NodeHandle, fn: (text: string | null) => void): void;
  onHover(node: NodeHandle, fn: () => void): void;
  onLeave(node: NodeHandle, fn: () => void): void;
  onWheel(node: NodeHandle, fn: (dx: number, dy: number) => void): void;
  onScroll(node: NodeHandle, fn: (x: number, y: number) => void): void;
  onSecondaryClick(node: NodeHandle, fn: () => void): void;
  textEdit(node: NodeHandle, op: number, text?: string): { text: string; start: number; end: number };
  onFrame(fn: (targetMs: number, intervalMs: number) => void): void;
  setDragClaim(node: NodeHandle, claim: "none" | "horizontal" | "free"): void;
  caretAt(
    node: NodeHandle,
    x: number,
    y: number,
  ): { unit: number; x: number; y: number; w: number; h: number };
  caretAtUnit(node: NodeHandle, unit: number): { x: number; y: number; w: number; h: number };
  selectionRects(
    node: NodeHandle,
    start: number,
    end: number,
  ): { x: number; y: number; w: number; h: number }[];
  wordRange(node: NodeHandle, unit: number): { start: number; end: number };
  focusStep(direction: "next" | "previous" | "left" | "right" | "up" | "down"): void;
  pushFocusScope(node: NodeHandle): void;
  popFocusScope(): void;
  pointerPosition(): { x: number; y: number };
  onBack(fn: () => boolean): number;
  offBack(id: number): void;
}
