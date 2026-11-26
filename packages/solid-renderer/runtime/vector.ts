import { Path2D } from "scene";
import type { CanvasRenderingContext2D, Path2D as VectorPath, VectorAsset, VectorIconDrawOptions } from "../src/canvas/types.ts";

type Stop = readonly [number, string];
type SolidPaint = string | readonly [fallback: string];
type Paint = SolidPaint
  | readonly [0, number, number, number, number, readonly Stop[]]
  | readonly [1, number, number, number, number, number, readonly Stop[]];
type Fill = readonly [Paint, opacity: number, evenOdd: boolean];
type Stroke<P = Paint> = readonly [P, opacity: number, width: number, cap: 0 | 1 | 2,
  join: 0 | 1 | 2, miterLimit: number, dashes?: readonly number[], dashOffset?: number];
type Operation = readonly [data: string, fill?: Fill | null, stroke?: Stroke | null,
  transform?: readonly [number, number, number, number, number, number] | null, strokeFirst?: boolean];

const caps = ["butt", "round", "square"] as const;
const joins = ["miter", "round", "bevel"] as const;

function tint(input: VectorIconDrawOptions["color"]): string | undefined {
  return typeof input === "string" || input === undefined
    ? input : `rgba(${input[0]},${input[1]},${input[2]},${input[3] / 255})`;
}

function brush(ctx: CanvasRenderingContext2D, paint: Paint, color: string | undefined) {
  if (typeof paint === "string") return paint;
  if (typeof paint[0] === "string") return color ?? paint[0];
  if (paint[0] === 0) {
    const gradient = ctx.createLinearGradient(paint[1], paint[2], paint[3], paint[4]);
    for (const [offset, value] of paint[5]) gradient.addColorStop(offset, value);
    return gradient;
  }
  const gradient = ctx.createRadialGradient(paint[1], paint[2], 0, paint[3], paint[4], paint[5]);
  for (const [offset, value] of paint[6]) gradient.addColorStop(offset, value);
  return gradient;
}

export function vectorAsset(width: number, height: number, operations: readonly Operation[], sharedStroke?: Stroke): VectorAsset {
  let paths: VectorPath[] | undefined;
  return {
    width,
    height,
    draw(ctx, options = {}) {
      const color = tint(options.color);
      const alpha = ctx.globalAlpha * (options.opacity ?? 1);
      paths ??= operations.map(([data]) => new Path2D(data));
      ctx.save();
      try {
        for (let index = 0; index < operations.length; index++) {
          const [, fill, localStroke, transform, strokeFirst] = operations[index]!;
          const stroke = localStroke ?? sharedStroke;
          const path = paths[index]!;
          if (transform) ctx.save();
          try {
            if (transform) ctx.transform(...transform);
            if (strokeFirst && stroke) strokePath(ctx, path, stroke, alpha, color);
            if (fill) {
              ctx.globalAlpha = alpha * fill[1];
              ctx.fillStyle = brush(ctx, fill[0], color);
              ctx.fill(path, fill[2] ? "evenodd" : "nonzero");
            }
            if (!strokeFirst && stroke) strokePath(ctx, path, stroke, alpha, color);
          } finally {
            if (transform) ctx.restore();
          }
        }
      } finally {
        ctx.restore();
      }
    },
  };
}

function strokePath(ctx: CanvasRenderingContext2D, path: VectorPath, stroke: Stroke, alpha: number, color: string | undefined) {
  configureStroke(ctx, stroke, alpha);
  ctx.strokeStyle = brush(ctx, stroke[0], color);
  ctx.stroke(path);
}

function configureStroke(ctx: CanvasRenderingContext2D, stroke: Stroke, alpha: number) {
  ctx.globalAlpha = alpha * stroke[1];
  ctx.lineWidth = stroke[2];
  ctx.lineCap = caps[stroke[3]];
  ctx.lineJoin = joins[stroke[4]];
  ctx.miterLimit = stroke[5];
  ctx.setLineDash(stroke[6] ?? []);
  ctx.lineDashOffset = stroke[7] ?? 0;
}

export function strokeAsset(width: number, height: number, data: readonly string[], stroke: Stroke<SolidPaint>): VectorAsset {
  let paths: VectorPath[] | undefined;
  return {
    width,
    height,
    draw(ctx, options = {}) {
      const paint = stroke[0];
      paths ??= data.map(path => new Path2D(path));
      ctx.save();
      try {
        configureStroke(ctx, stroke, ctx.globalAlpha * (options.opacity ?? 1));
        ctx.strokeStyle = typeof paint === "string" ? paint : tint(options.color) ?? paint[0];
        for (const path of paths) ctx.stroke(path);
      } finally {
        ctx.restore();
      }
    },
  };
}
