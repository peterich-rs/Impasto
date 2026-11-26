import type {
  CanvasBinding,
  CanvasError,
  CanvasRenderingContext2D,
  CanvasSize,
  Path2DConstructor,
} from "./canvas/types.ts";
import type { StyleInput } from "./style.ts";

export type AutofillHint = "username" | "password" | "newUsername" | "newPassword" | "emailAddress";

/** Stable scene-node handle. 0 means none, at the QuickJS boundary only. */
export type NodeHandle = number;

/** Platform settings projected into the script. Field names match the session. */
export type PlatformSnapshot = {
  locale: string;
  textScale: number;
  brightness: "light" | "dark";
  reduceMotion: boolean;
  boldText: boolean;
  highContrast: boolean;
};

export type ScrollChannel = "scrollX" | "scrollY" | "overscrollX" | "overscrollY";

type WindowInsets = { top: number; right: number; bottom: number; left: number };

export type WindowMetrics = {
  /** Display information, when supplied by the host. */
  displayId?: number;
  refreshRate?: number;
  /** Coordinates in logical points relative to the current Impasto View. */
  displayFeatures?: readonly DisplayFeature[];
  width: number;
  height: number;
  pixelWidth: number;
  pixelHeight: number;
  devicePixelRatio: number;
  padding: WindowInsets;
  viewInsets: WindowInsets;
  /** Local obstruction such as an undocked iPad keyboard. */
  keyboardOcclusion?: { x: number; y: number; width: number; height: number };
  systemGestureInsets: WindowInsets;
};
export type DisplayFeature = {
  x: number;
  y: number;
  width: number;
  height: number;
  type: "fold" | "hinge" | "cutout";
  posture?: "flat" | "halfOpened";
  separating: boolean;
};

export type GradientStopData = {
  offset: number;
  color: string | [number, number, number, number];
};

export type SpanGradient = {
  kind: "linear" | "radial" | "sweep";
  from?: [number, number];
  to?: [number, number];
  center?: [number, number];
  radius?: number;
  startAngle?: number;
  endAngle?: number;
  stops: GradientStopData[];
};

/** One structured text run, with optional foreground and gradient. */
export type TextSpanData = {
  text: string;
  color?: string | [number, number, number, number];
  weight?: number;
  family?: string;
  underline?: boolean;
  lineThrough?: boolean;
  gradient?: SpanGradient;
};

/**
 * The named `scene` module exposed by the Rust QuickJS bindings.
 * Handles are JS numbers. A live id is never 0.
 */
export interface SceneModule {
  createElement(tag: string, props?: Record<string, unknown>): NodeHandle;
  createTextNode(value: string | number): NodeHandle;
  setProp(node: NodeHandle, name: string, value: unknown): void;
  insertNode(parent: NodeHandle, child: NodeHandle, before?: NodeHandle): void;
  createStyleSource(
    node: NodeHandle,
    origin: "component" | "classes" | "inline" | "imperative" | "animation",
  ): number;
  setStyleSource(source: number, style: StyleInput | string): void;
  removeStyleSource(source: number): void;
  setStyle(node: NodeHandle, style: StyleInput): void;
  patchStyle(node: NodeHandle, style: StyleInput): void;
  clearStyle(node: NodeHandle, keys?: string[]): void;
  setClass(node: NodeHandle, text: string): void;
  setComponentStyle(node: NodeHandle, style: StyleInput): void;
  append(parent: NodeHandle, child: unknown): void;
  insert(parent: NodeHandle, children: unknown, before?: NodeHandle | null): void;
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
  setTextRuns(node: NodeHandle, runs: TextSpanData[]): void;
  placePanel(panel: NodeHandle, anchor: NodeHandle, align: number): void;
  reparentChildren(from: NodeHandle, to: NodeHandle): void;
  textCommand(
    node: NodeHandle,
    name: string,
    extra?: string,
    recordUndo?: boolean,
  ): { text: string; start: number; end: number };
  setTextLimit(node: NodeHandle, limit: number): void;
  setImage(
    node: NodeHandle,
    source: string,
    width?: number,
    height?: number,
    required?: boolean,
  ): void;
  /** `"list-content"` restricts author layout on a driver-owned content node. */
  setStyleSurface(
    node: NodeHandle,
    surface: "general" | "stack" | "scroll-viewport" | "list-content",
  ): void;
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
    /** `0` content, `1` always, `2` never. Omitted is content. */
    accept?: number,
    /** `0` tau in milliseconds, `1` deceleration rate per millisecond. */
    decayKind?: number,
    snapStiffness?: number,
    snapDamping?: number,
    /** `0` linear factor, `1` asymptotic curve. */
    bandKind?: number,
  ): void;
  setScrollChain(node: NodeHandle, chain: boolean): void;
  fling(node: NodeHandle, vx: number, vy: number): void;
  stopScroll(node: NodeHandle): void;
  scrollTo(
    node: NodeHandle,
    x: number,
    y: number,
    stiffness: number,
    damping: number,
  ): void;
  scrollOffset(node: NodeHandle): { x: number; y: number };
  rect(node: NodeHandle): { x: number; y: number; w: number; h: number };
  setFocusable(node: NodeHandle, enabled: boolean): void;
  setRepaintBoundary?(node: NodeHandle, enabled: boolean): void;
  setTransitionSnapshot?(node: NodeHandle, enabled: boolean): void;
  setColorFilter?(node: NodeHandle, matrix: readonly number[] | null): void;
  setBackdropMaterial?(node: NodeHandle, parameters: readonly number[] | null): void;
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
  /** Replace the session theme snapshot while preserving window dimensions. */
  setStyleEnvironment(environment: StyleEnvironment): void;
  setPlatformView(node: NodeHandle, kind: string | null, clipPath?: string): void;
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
    hint?: { order?: number; live?: "off" | "polite" | "assertive"; password?: boolean; autofill?: { hint: AutofillHint; value: string } },
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
  registerFont(family: string, bytes: Uint8Array): void;
  registerFontAsset(family: string, assetUri: string): void;
  /** Registers WGSL for this session and returns a handle to the source. */
  registerShaderProgram(source: string, animated?: boolean): number;
  /** Applies a program to the node's content box. Passing null clears it. */
  setShaderEffect(
    node: NodeHandle,
    program: number | null,
    uniforms?: Uint8Array,
    textures?: readonly string[],
  ): void;
  animateShaderUniform(
    node: NodeHandle,
    index: number,
    target: number,
    durationMs: number,
    easing?: "linear" | "ease-in-out",
  ): void;
  reload(source: string): void;
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
  onScroll(node: NodeHandle, fn?: (x: number, y: number) => void): void;
  observeScrollOffset(node: NodeHandle, fn?: (x: number, y: number) => void): void;
  bindScrollProp(
    source: NodeHandle, channel: ScrollChannel, target: NodeHandle, prop: string,
    inputs: readonly number[] | number, outputs: readonly number[] | readonly string[] | number,
    extrapolate: "clamp" | "extend" | "linear",
  ): number;
  bindBackProgressOpacity(target: NodeHandle, inputs: readonly number[], outputs: readonly number[]): number;
  unbindScrollProp(id: number): void;
  addScrollThreshold(node: NodeHandle, source: ScrollChannel, value: number, fromEnd: boolean, hysteresis: number): number;
  removeScrollThreshold(id: number): void;
  onScrollThreshold(fn: (id: number, active: boolean) => void): void;
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
  onLongPress(node: NodeHandle, fn: () => void): void;
  textEdit(
    node: NodeHandle,
    op: number,
    text?: string,
  ): { text: string; start: number; end: number };
  onFrame(fn?: (targetMs: number, intervalMs: number) => void): void;
  /** One callback, installed by `boot`, run after layout. */
  onLayout(fn: () => void): void;
  setDragClaim(node: NodeHandle, claim: "none" | "horizontal" | "free"): void;
  caretAt(
    node: NodeHandle,
    x: number,
    y: number,
  ): { unit: number; x: number; y: number; w: number; h: number };
  caretAtUnit(
    node: NodeHandle,
    unit: number,
  ): { x: number; y: number; w: number; h: number };
  selectionRects(
    node: NodeHandle,
    start: number,
    end: number,
  ): { x: number; y: number; w: number; h: number }[];
  wordRange(node: NodeHandle, unit: number): { start: number; end: number };
  focusStep(
    direction: "next" | "previous" | "left" | "right" | "up" | "down",
  ): void;
  pushFocusScope(node: NodeHandle): void;
  /** Remove this node's scope and report whether it was active. Omit the node to pop the top scope. */
  popFocusScope(node?: NodeHandle): boolean;
  pointerPosition(): { x: number; y: number };
  penSample(): {
    id: string;
    phase: "down" | "move" | "up" | "cancel";
    x: number;
    y: number;
    time: number;
    force: number;
    radius: number;
    altitude: number;
    azimuth: number;
  } | null;
  onBack(fn: () => boolean): number;
  offBack(id: number): void;
  onBackGesture?(fn: (phase: "start" | "progress" | "cancel" | "commit", value: number) => boolean | void): void;
  setBackEnabled?(enabled: boolean): void;
  setSemanticsHidden?(node: NodeHandle, hidden: boolean): void;
  onMotionEnd(id: NodeHandle, prop: string, fn: () => void): number;
  offMotionEnd(id: number): void;
  /** Pinch scale along the hit path. Older hosts omit it. */
  onZoom(node: NodeHandle, fn: (scale: number) => void): void;
  /** Spec is `mod+k` style. Older hosts omit it. */
  onShortcut(spec: string, cb: () => void): void;
  scrollIntoView(id: NodeHandle, edge: number): void;
  /**
   * Spring on one property. `prop` is `scrollX`, `scrollY`, `opacity`,
   * `translateX`, `translateY`, `rotate`, `scale`, `overscrollX`, or `overscrollY`.
   * Core calls opacity `Opacity`; the script string is `opacity`.
   * With `retarget`, a running motion supplies the starting value.
   */
  animate(
    id: NodeHandle,
    prop: string,
    from: number,
    to: number,
    stiffness: number,
    damping: number,
    retarget?: boolean,
  ): void;
  flingTarget(id: NodeHandle): { x: number; y: number } | null;
  setListenerErrorPolicy(policy: "report" | "freeze"): void;
  layerTreeStats(): { epoch: number; layers: number; pictures: number };
  /**
   * Optional recognizers. `doubleTapMs` is the double-tap window in milliseconds.
   */
  setRecognizers(
    node: NodeHandle,
    spec: {
      tap?: boolean;
      longPress?: boolean;
      drag?: "none" | "free" | "horizontal" | "vertical";
      doubleTapMs?: number;
    },
  ): void;
  log(level: string, message: string): void;
  /** Overflow report JSON. Present when the scene linked the debug bindings. */
  debugLayoutIssues(): string | null;
  debugSnapshot(): string | null;
  debugProbe(x: number, y: number): string | null;
  debugLastPointer(): string | null;
  debugSetScan(on: boolean): void;
  debugSetPointerTrace(on: boolean, suppress: boolean): void;
  debugSetOwned(node: NodeHandle, on: boolean): void;
  debugSetSource(node: NodeHandle, path: string, line: number, column: number, component: string): void;
  debugMemory(): string | null;
  metrics(): {
    width: number;
    height: number;
    viewInsets: { top: number; right: number; bottom: number; left: number };
  };
  /** Full host View dimensions and insets in scene points, plus its pixel extent. */
  windowMetrics(): WindowMetrics;
  onWindowMetrics(fn: (metrics: WindowMetrics) => void): number;
  offWindowMetrics(id: number): void;
  /** Native list state machine. The skin applies render ops. */
  listDriver(overscan: number, estimate: number): ListDriver;
  /** Native scroll clamp, drag, and refresh threshold. */
  scrollController(): ScrollDriver;
  /**
   * One callback for every driver. `kind` 1 is a list, 2 is a scroll.
   * Installed once by `boot`.
   */
  onDriver(
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
  /** Records the content box. A thenable is AsyncDraw. An error handler can return false to use diagnostics. */
  bindCanvas(
    node: NodeHandle,
    draw: (context: CanvasRenderingContext2D, size: CanvasSize) => unknown,
    onDrawError?: (error: CanvasError) => void,
    deferWhenOffscreen?: boolean,
  ): CanvasBinding;
  Path2D: Path2DConstructor;
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

/** Native theme tokens shared by object styles and utility classes. */
export interface StyleEnvironment {
  direction: "ltr" | "rtl";
  dark: boolean;
  spacing: number;
  rootRem: number;
  colors: Readonly<Record<string, string>>;
  lengths?: Readonly<Record<string, number>>;
  radii: readonly [number, number, number, number, number, number, number];
  fontSizes: readonly [number, number, number, number, number, number, number, number];
  roles: Readonly<Record<"display" | "headline" | "title" | "body" | "label" | "caption", {
    size: number; weight: number; lineHeight: number;
  }>>;
}
