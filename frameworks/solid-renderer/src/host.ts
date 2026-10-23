/** Stable scene-node handle. 0 means none, at the QuickJS boundary only. */
export type NodeHandle = number;

/**
 * The `core` object QuickJS installs. Method names match `glue.c`.
 * Handles are JS numbers. A live id is never 0.
 */
export interface CoreHost {
  column(gap: number): NodeHandle;
  row(gap: number): NodeHandle;
  stack(): NodeHandle;
  padding(edges: number): NodeHandle;
  pointerRegion(): NodeHandle;
  decorated(color: number[], radius: number): NodeHandle;
  clip(radius: number): NodeHandle;
  opacity(alpha: number): NodeHandle;
  scroll(): NodeHandle;
  transform(): NodeHandle;
  image(source: string, width: number, height: number): NodeHandle;
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
  setGrow(node: NodeHandle, grow: number): void;
  setRadius(node: NodeHandle, radius: number): void;
  setAlpha(node: NodeHandle, alpha: number): void;
  setAlign(node: NodeHandle, align: "start" | "center" | "end"): void;
  setSize(node: NodeHandle, width: number, height: number): void;
  setImage(node: NodeHandle, source: string, width: number, height: number): void;
  setMaxLines(node: NodeHandle, lines: number): void;
  setEllipsis(node: NodeHandle, text: string): void;
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
  blur(node: NodeHandle): void;
  setImeEnabled(enabled: boolean): void;
  setImeText(text: string): void;
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
  onComposing(node: NodeHandle, fn: (text: string | null) => void): void;
  onFrame(fn: (timeMs: number) => void): void;
}
