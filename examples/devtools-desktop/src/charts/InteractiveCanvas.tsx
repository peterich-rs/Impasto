import { Show, createSignal, createTrackedEffect, onCleanup } from "solid-js";
import { Canvas, RepaintBoundary, View, type CanvasController, type CanvasRenderingContext2D } from "@impasto/solid-ui";
import { onLayout, scene, type NodeHandle } from "@impasto/solid-renderer";
import type { ProjectedCanvas } from "@app/devtools-core";
import { Label } from "../components/ui";
import { useTokens } from "../theme/tokens";

type Position = { x: number; y: number; width: number; height: number };
export type PlotSize = Pick<Position, "width" | "height">;

function f64(input: Uint8Array | ArrayBuffer): Float64Array {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (bytes.byteLength % 8 !== 0) return new Float64Array();
  return bytes.byteOffset % 8 === 0
    ? new Float64Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 8)
    : new Float64Array(bytes.slice().buffer);
}

export function paint(ctx: CanvasRenderingContext2D, plot: ProjectedCanvas, colors: readonly string[], fallback: string) {
  for (const { color, rects } of plot.rectBuckets) {
    if (rects.byteLength === 0) continue;
    ctx.fillStyle = colors[color] ?? fallback;
    ctx.fillRects(f64(rects));
  }
  for (const { color, width, dashed, points } of plot.segments) {
    if (points.byteLength === 0) continue;
    ctx.lineWidth = width;
    ctx.strokeStyle = colors[color] ?? fallback;
    ctx.setLineDash(dashed ? [4, 4] : []);
    ctx.beginPath();
    ctx.lineSegments(f64(points));
    ctx.stroke();
  }
  ctx.setLineDash([]);
  for (const { color, width, points } of plot.polylines) {
    if (points.byteLength < 32) continue;
    ctx.lineWidth = width;
    ctx.strokeStyle = colors[color] ?? fallback;
    ctx.beginPath();
    ctx.polyline(f64(points));
    ctx.stroke();
  }
  ctx.font = "11px Monospace";
  ctx.textBaseline = "middle";
  for (const label of plot.labels) {
    if (label.maxWidth <= 0) continue;
    ctx.fillStyle = colors[label.color] ?? fallback;
    ctx.textAlign = label.align === 1 ? "center" : label.align === 2 ? "right" : "left";
    ctx.fillText(label.text, label.x, label.y, label.maxWidth);
  }
}

export function InteractiveCanvas(props: {
  label: string;
  height: number;
  project: (width: number, height: number) => ProjectedCanvas | undefined;
  onSize?: (size: PlotSize) => void;
  onClick?: (point: Position) => void;
  onPress?: (point: Position) => void;
  onHover?: (point: Position) => void;
  onLeave?: () => void;
  onDrag?: (point: Position, dx: number, dy: number) => void;
  onDragEnd?: () => void;
  onWheel?: (dx: number, dy: number, size: PlotSize) => void;
  onZoom?: (scale: number, size: PlotSize) => void;
  onKey?: (key: string) => void;
  onReset?: () => void;
  tip?: string;
  cursor?: number;
  interactive?: boolean;
}) {
  const colors = useTokens();
  const [size, setSize] = createSignal<PlotSize>({ width: 0, height: 0 });
  const [position, setPosition] = createSignal<Position>();
  const [dragging, setDragging] = createSignal(false);
  let binding: CanvasController | null = null;
  let node: NodeHandle | null = null;
  onCleanup(onLayout(() => {
    if (!node) return;
    const { w, h } = scene.rect(node);
    const old = size();
    if (w === old.width && h === old.height) return;
    const next = { width: w, height: h };
    setSize(next);
    props.onSize?.(next);
  }));
  let projected: ProjectedCanvas | undefined;
  let palette = colors();
  // Native plugin calls belong to a reactive effect. The paint callback only
  // reads the completed projection while the Canvas recorder owns the scene.
  createTrackedEffect(() => {
    const { width, height } = size();
    const nextPalette = colors();
    const nextPlot = width > 0 && height > 0 ? props.project(width, height) : undefined;
    if (nextPlot === projected && nextPalette === palette) return;
    palette = nextPalette;
    projected = nextPlot;
    binding?.invalidate();
  });
  const pointer = () => {
    const p = binding?.toLocal(scene.pointerPosition());
    if (!p) return;
    return { ...p, ...size() };
  };
  return <View
    accessibilityLabel={props.label}
    focusable={!!props.onKey}
    style={{ width: "100%", height: props.height, position: "relative", flexShrink: 0,
      backgroundColor: colors().canvas, overflow: "hidden",
      cursor: dragging() ? "grabbing" : props.interactive ? "grab" : "default" }}
    ref={element => { if (element && props.onDrag) scene.setDragClaim(element, "horizontal"); }}
    onKey={props.onKey}
    onPress={() => { const p = pointer(); if (p) props.onPress?.(p); }}
    onClick={() => { const p = pointer(); if (p) props.onClick?.(p); }}
    onHover={() => { const p = pointer(); if (p) { if (props.tip !== undefined) setPosition(p); props.onHover?.(p); } }}
    onLeave={() => { setPosition(undefined); props.onLeave?.(); }}
    onDragBegin={() => setDragging(true)}
    onDragBy={(dx, dy) => { const p = pointer(); if (p) props.onDrag?.(p, dx, dy); }}
    onDragEnd={() => { setDragging(false); props.onDragEnd?.(); }}
    onWheel={(dx, dy) => props.onWheel?.(dx, dy, size())}
    onZoom={scale => props.onZoom?.(scale, size())}
    onDoubleTap={() => props.onReset?.()}>
    <Canvas
      ref={element => { node = element; }}
      deferWhenOffscreen
      style={{ width: "100%", height: "100%" }}
      controller={value => { binding = value; }}
      onDrawError={error => {
        console.error(`${props.label} · ${error.api ?? error.code} · ${error.message}`);
      }}
      draw={ctx => { if (projected) paint(ctx, projected, palette.buckets, palette.primary); }}
    />
    <Show when={props.cursor != null}>
      <RepaintBoundary style={{ position: "absolute", width: 1, height: props.height, top: 0, left: 0,
        translateX: (props.cursor ?? 0) * size().width, backgroundColor: colors().secondary }} />
    </Show>
    <Show when={props.tip && position()}>
      <RepaintBoundary style={{ position: "absolute", left: 0, top: 0, width: Math.min(300, size().width),
        translateX: Math.min((position()?.x ?? 0) + 12, Math.max(0, size().width - 300)),
        translateY: Math.min(Math.max(0, (position()?.y ?? 0) + 12), Math.max(0, props.height - 26)),
        padding: 6, backgroundColor: colors().elevated, borderRadius: 4 }}>
        <Label size={11} mono>{props.tip}</Label>
      </RepaintBoundary>
    </Show>
  </View>;
}
