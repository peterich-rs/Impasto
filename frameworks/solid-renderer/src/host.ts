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
  align(align: "start" | "center" | "end"): NodeHandle;
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
  /** Unlink `child` and keep its id. Does not queue retirement. */
  detach(parent: NodeHandle, child: NodeHandle): void;
  /** Retire a parentless node and its subtree. */
  discard(node: NodeHandle): void;
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
  setCross(
    node: NodeHandle,
    align: "start" | "center" | "end" | "stretch" | "baseline",
  ): void;
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
  overscroll(node: NodeHandle): { x: number; y: number };
  setScrollPhysics(
    node: NodeHandle,
    tau: number,
    mode: number,
    stiffness: number,
    damping: number,
    factor: number,
    snapX: number,
    alignX: number,
    snapY: number,
    alignY: number,
  ): void;
  setScrollChain(node: NodeHandle, chain: boolean): void;
  fling(node: NodeHandle, vx: number, vy: number): void;
  stopScroll(node: NodeHandle): void;
  scrollTo(node: NodeHandle, x: number, y: number, stiffness: number, damping: number): void;
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
    hint?: { order?: number; live?: "off" | "polite" | "assertive" },
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
  /** One callback, installed by `boot`. Phase is `idle` | `dragging` | `coasting` | `settling`. */
  onScrollState(
    fn: (
      node: NodeHandle,
      phase: number,
      vx: number,
      vy: number,
      ox: number,
      oy: number,
    ) => void,
  ): void;
  onSecondaryClick(node: NodeHandle, fn: () => void): void;
  textEdit(node: NodeHandle, op: number, text?: string): { text: string; start: number; end: number };
  onFrame(fn: (targetMs: number, intervalMs: number) => void): void;
  /** One callback, installed by `boot`, run after layout. */
  onLayout(fn: () => void): void;
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
  onMotionEnd?(id: NodeHandle, prop: string, fn: () => void): number;
  offMotionEnd?(id: number): void;
  /** Pinch scale along the hit path. Older hosts omit it. */
  onZoom?(node: NodeHandle, fn: (scale: number) => void): void;
  /** Spec is `mod+k` style. Older hosts omit it. */
  onShortcut?(spec: string, cb: () => void): void;
  scrollIntoView?(id: NodeHandle, edge: number): void;
  /**
   * Spring on one property. `prop` is `scrollX`, `scrollY`, `opacity`,
   * `translateX`, `translateY`, `rotate`, `scale`, `overscrollX`, or `overscrollY`.
   * Core calls opacity `Opacity`; the script string is `opacity`.
   */
  animate?(
    id: NodeHandle,
    prop: string,
    from: number,
    to: number,
    stiffness: number,
    damping: number,
  ): void;
  flingTarget?(id: NodeHandle): { x: number; y: number } | null;
  setListenerErrorPolicy?(policy: "report" | "freeze"): void;
  layerTreeStats?(): { epoch: number; layers: number; pictures: number };
  /**
   * Optional recognizers. `doubleTapMs` is the double-tap window in milliseconds.
   */
  setRecognizers?(
    node: NodeHandle,
    spec: {
      tap?: boolean;
      longPress?: boolean;
      drag?: "none" | "free" | "horizontal" | "vertical";
      doubleTapMs?: number;
    },
  ): void;
  setStroke?(node: NodeHandle, width: number, color: number[]): void;
  log?(level: string, message: string): void;
  metrics?(): {
    width: number;
    height: number;
    viewInsets: { top: number; right: number; bottom: number; left: number };
  };
  /** Native list state machine. The skin applies render ops. */
  listDriver?(overscan: number, estimate: number): ListDriver;
  /** Native scroll clamp, drag, and refresh threshold. */
  scrollController?(): ScrollDriver;
  /**
   * One callback for every driver. `kind` 1 is a list, 2 is a scroll.
   * Installed once by `boot`.
   */
  onDriver?(
    fn: (
      kind: number,
      index: number,
      generation: number,
      tag: number,
      i0: number,
      i1: number,
      f0: number,
      f1: number,
      f2: number,
      f3: number,
      f4: number,
      f5: number,
      s0: string,
      s1: string,
    ) => void,
  ): void;
}

/** Handle for `core.listDriver`. Methods stay on the QuickJS class. */
export interface ListDriver {
  bind(
    viewport: number,
    content: number,
    top: number,
    bottom: number,
    rows: number,
    vThumb: number,
    hThumb: number,
    pin: number,
    vSize: number,
    hSize: number,
  ): void;
  configure(estimate: number, overscan: number, blocked: boolean): void;
  setItems(rows: { key: string; type: string; sticky: boolean }[]): void;
  noteRow(key: string, node: number): void;
  release(): void;
  id(): { index: number; generation: number };
}

/** Handle for `core.scrollController`. */
export interface ScrollDriver {
  bind(viewport: number, content: number): void;
  configure(
    contentMin: number,
    pullMode: number,
    threshold: number,
    rubber: boolean,
    factor: number,
    parent: ScrollDriver | null,
  ): void;
  dragBy(
    dx: number,
    dy: number,
  ): { ox: number; oy: number; overX: number; overY: number; pending: boolean };
  finishDrag(taken: boolean): void;
  dragEnd(): void;
  wheel(dx: number, dy: number): void;
  query(cmd: number): { x: number; y: number };
  consume(ox: number, oy: number): boolean;
  release(): void;
  id(): { index: number; generation: number };
}
