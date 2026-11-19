/** Screen canvas types. Only methods the runtime implements are declared. */

export type Matrix2D = Readonly<{
  a: number;
  b: number;
  c: number;
  d: number;
  e: number;
  f: number;
}>;

export type FillRule = "nonzero" | "evenodd";

export type CanvasPaint = string | CanvasGradient;

export type CanvasComposite =
  | "source-over"
  | "destination-over"
  | "destination-out"
  | "multiply"
  | "screen"
  | "lighter"
  | "xor";

export interface CanvasSize {
  readonly width: number;
  readonly height: number;
  readonly pixelRatio: number;
  readonly textScale: number;
  readonly reduceMotion: boolean;
}

export interface CanvasError {
  name: "CanvasError";
  code: string;
  message: string;
  api?: string;
}

export interface CanvasPath {
  closePath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  quadraticCurveTo(cx: number, cy: number, x: number, y: number): void;
  bezierCurveTo(
    c1x: number,
    c1y: number,
    c2x: number,
    c2y: number,
    x: number,
    y: number,
  ): void;
  rect(x: number, y: number, width: number, height: number): void;
  roundRect(
    x: number,
    y: number,
    width: number,
    height: number,
    radii?: number | readonly number[],
  ): void;
  arc(
    x: number,
    y: number,
    radius: number,
    start: number,
    end: number,
    counterclockwise?: boolean,
  ): void;
  arcTo(x1: number, y1: number, x2: number, y2: number, radius: number): void;
  ellipse(
    x: number,
    y: number,
    rx: number,
    ry: number,
    rotation: number,
    start: number,
    end: number,
    counterclockwise?: boolean,
  ): void;
}

export interface CanvasRenderingContext2D extends CanvasPath {
  readonly canvas: Readonly<{ width: number; height: number }>;
  fillStyle: CanvasPaint;
  strokeStyle: CanvasPaint;
  lineWidth: number;
  lineCap: "butt" | "round" | "square";
  lineJoin: "miter" | "round" | "bevel";
  miterLimit: number;
  globalAlpha: number;
  globalCompositeOperation: CanvasComposite;
  lineDashOffset: number;
  font: string;
  textAlign: "left" | "right" | "center" | "start" | "end";
  textBaseline: "alphabetic" | "ideographic" | "top" | "middle" | "bottom";
  direction: "ltr" | "rtl" | "inherit";
  imageSmoothingEnabled: boolean;
  save(): void;
  restore(): void;
  beginPath(): void;
  /** Appends one new subpath from x/y pairs. */
  polyline(points: Float64Array): void;
  /** Appends one subpath per x0/y0/x1/y1 group. */
  lineSegments(points: Float64Array): void;
  /** Fills x/y/width/height rectangles in order with the current paint. */
  fillRects(rects: Float64Array): void;
  translate(x: number, y: number): void;
  rotate(radians: number): void;
  scale(x: number, y: number): void;
  transform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  resetTransform(): void;
  getTransform(): Matrix2D;
  setLineDash(segments: readonly number[]): void;
  getLineDash(): number[];
  fill(rule?: FillRule): void;
  fill(path: Path2D, rule?: FillRule): void;
  stroke(path?: Path2D): void;
  clip(rule?: FillRule): void;
  clip(path: Path2D, rule?: FillRule): void;
  fillRect(x: number, y: number, width: number, height: number): void;
  strokeRect(x: number, y: number, width: number, height: number): void;
  clearRect(x: number, y: number, width: number, height: number): void;
  createLinearGradient(x0: number, y0: number, x1: number, y1: number): CanvasGradient;
  createRadialGradient(
    x0: number,
    y0: number,
    r0: number,
    x1: number,
    y1: number,
    r1: number,
  ): CanvasGradient;
  createConicGradient(angle: number, x: number, y: number): CanvasGradient;
  fillText(text: string, x: number, y: number, maxWidth?: number): void;
  measureText(text: string): TextMetrics;
  drawImage(source: string, dx: number, dy: number): void;
  drawImage(source: string, dx: number, dy: number, dw: number, dh: number): void;
  drawImage(
    source: string,
    sx: number,
    sy: number,
    sw: number,
    sh: number,
    dx: number,
    dy: number,
    dw: number,
    dh: number,
  ): void;
  isPointInPath(x: number, y: number, rule?: FillRule): boolean;
  isPointInPath(path: Path2D, x: number, y: number, rule?: FillRule): boolean;
  isPointInStroke(x: number, y: number): boolean;
  isPointInStroke(path: Path2D, x: number, y: number): boolean;
}

export interface CanvasGradient {
  addColorStop(offset: number, color: string): void;
}

export interface TextMetrics {
  readonly width: number;
  readonly fontBoundingBoxAscent: number;
  readonly fontBoundingBoxDescent: number;
}

export interface Path2D extends CanvasPath {
  addPath(path: Path2D, transform?: Matrix2D): void;
}

export interface Path2DConstructor {
  new (path?: Path2D | string): Path2D;
}

/** Native binding. `release` is for the component owner, not the public controller. */
export interface CanvasBinding {
  invalidate(): void;
  toLocal(point: { x: number; y: number }): { x: number; y: number } | null;
  release(): void;
}
