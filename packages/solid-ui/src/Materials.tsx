import { createTrackedEffect } from "solid-js";
import { scene, type NodeHandle, type StyleInput } from "@impasto/solid-renderer";
import { bindBox } from "./element.tsx";
import type { ViewProps } from "./props.ts";
import { createShaderProgram, ShaderEffect } from "./ShaderEffect.tsx";
import { View } from "./View.tsx";

/** Straight-alpha linear sRGB channels. */
export type MaterialColor = readonly [red: number, green: number, blue: number, alpha: number];

function packed(values: readonly number[]): Uint8Array {
  const bytes = new Uint8Array(values.length * 4);
  const view = new DataView(bytes.buffer);
  values.forEach((value, index) => view.setFloat32(index * 4, Number.isFinite(value) ? value : 0, true));
  return bytes;
}

const shimmerShader = `
fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let x = local_pos.x / max(1.0, size.x);
    let center = fract(effect_time() * effect_uniform(0u)) * 1.8 - 0.4;
    let highlight = 1.0 - smoothstep(0.0, 0.34, abs(x + local_pos.y / max(size.y, 1.0) * 0.18 - center));
    let base = vec4(effect_uniform(1u), effect_uniform(2u), effect_uniform(3u), effect_uniform(4u));
    let light = vec4(effect_uniform(5u), effect_uniform(6u), effect_uniform(7u), effect_uniform(8u));
    return mix(base, light, highlight);
}`;

export interface ShimmerProps extends Omit<ViewProps, "children"> {
  base?: MaterialColor;
  highlight?: MaterialColor;
  periodMs?: number;
  animated?: boolean;
}

/** Draws a skeleton placeholder with the raster clock. */
export function Shimmer(props: ShimmerProps): NodeHandle {
  const animated = createShaderProgram(shimmerShader, { animated: true });
  const still = createShaderProgram(shimmerShader);
  return (
    <ShaderEffect {...props} program={props.animated !== false && !scene.reduceMotion() ? animated : still}
      uniforms={packed([
        1000 / Math.max(250, props.periodMs ?? 1400),
        ...(props.base ?? [0.71, 0.75, 0.78, 1]),
        ...(props.highlight ?? [0.91, 0.93, 0.95, 1]),
      ])} />
  );
}

const rippleShader = `
fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let progress = clamp(effect_uniform(0u), 0.0, 1.0);
    let center = vec2(effect_uniform(1u), effect_uniform(2u)) * size;
    let radius = max(length(center), length(size - center)) * progress * 1.5;
    let coverage = 1.0 - smoothstep(radius - 1.0, radius + 1.0, distance(center, local_pos));
    let color = vec3(effect_uniform(3u), effect_uniform(4u), effect_uniform(5u));
    return vec4(color, effect_uniform(6u) * coverage * (1.0 - progress));
}`;

export interface RippleProps extends ViewProps {
  color?: MaterialColor;
  durationMs?: number;
}

/** Adds a centered press ripple. One input event starts the compositor animation. */
export function Ripple(props: RippleProps): NodeHandle {
  const program = createShaderProgram(rippleShader);
  let surface: NodeHandle | null = null;
  const onPress = () => {
    props.onPress?.();
    if (surface === null || scene.reduceMotion()) return;
    scene.setShaderEffect(surface, program, packed([0, 0.5, 0.5, ...(props.color ?? [1, 1, 1, 0.25])]));
    scene.animateShaderUniform(surface, 0, 1, Math.max(100, props.durationMs ?? 420), "linear");
  };
  const clipped = (): StyleInput => Array.isArray(props.style)
    ? [...props.style, { overflow: "hidden" }]
    : [props.style, { overflow: "hidden" }] as StyleInput;
  return (
    <View {...props} style={clipped()} onPress={onPress}>
      {props.children}
      <view ref={(node) => {
        surface = node;
        bindBox(node, {}, { position: "absolute", top: 0, left: 0, width: "100%", height: "100%" });
      }} />
    </View>
  );
}

export interface VibrancyProps extends Omit<ViewProps, "children"> {
  radius?: number;
  saturation?: number;
  brightness?: number;
  tint?: MaterialColor;
  /** Fraction of the blur that fades into the sharp backdrop at the bottom. */
  progressive?: number;
}

/** Blurs and color-treats the backdrop before drawing in this box. */
export function Vibrancy(props: VibrancyProps): NodeHandle {
  return (
    <View {...props} ref={(node) => {
      if (node !== null) {
        createTrackedEffect(() => scene.setBackdropMaterial?.(node, [
          Math.min(64, Math.max(0, props.radius ?? 7)),
          Math.max(0, props.saturation ?? 1.25),
          Math.max(0, props.brightness ?? 1.08),
          props.progressive ?? 0,
          ...(props.tint ?? [1, 1, 1, 0.12]),
        ]));
      }
      props.ref?.(node);
    }} />
  );
}

const insetShadowShader = `
fn rounded_distance(p: vec2<f32>, extent: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - extent + radius;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let half = size * 0.5;
    let corner = clamp(effect_uniform(3u), 0.0, min(half.x, half.y));
    let center = local_pos - half;
    let clip = 1.0 - smoothstep(-0.5, 0.5, rounded_distance(center, half, corner));
    let depth = -rounded_distance(center - vec2(effect_uniform(1u), effect_uniform(2u)), half, corner);
    let shadow = (1.0 - smoothstep(0.0, max(1.0, effect_uniform(0u)), depth)) * clip;
    let color = vec3(effect_uniform(4u), effect_uniform(5u), effect_uniform(6u));
    return vec4(color, shadow * effect_uniform(7u));
}`;

export interface InsetShadowProps extends Omit<ViewProps, "children"> {
  radius?: number;
  offset?: readonly [x: number, y: number];
  cornerRadius?: number;
  color?: MaterialColor;
}

/** Paints an inner shadow over the contents of its layout box. */
export function InsetShadow(props: InsetShadowProps): NodeHandle {
  const program = createShaderProgram(insetShadowShader);
  return (
    <ShaderEffect {...props} program={program} uniforms={packed([
      Math.max(1, props.radius ?? 8),
      ...(props.offset ?? [0, 2]),
      props.cornerRadius ?? 0,
      ...(props.color ?? [0, 0, 0, 0.25]),
    ])} />
  );
}

/** Four rows of linear RGBA coefficients followed by the row's bias. */
export type ColorMatrix = readonly [
  readonly [number, number, number, number, number],
  readonly [number, number, number, number, number],
  readonly [number, number, number, number, number],
  readonly [number, number, number, number, number],
];

export interface ColorFilterProps extends ViewProps {
  matrix?: ColorMatrix;
}

/** Applies a straight linear RGBA matrix to the composited children. */
export function ColorFilter(props: ColorFilterProps): NodeHandle {
  return <View {...props} ref={(node) => {
    if (node !== null) {
      createTrackedEffect(() => scene.setColorFilter?.(node, props.matrix?.flat() ?? null));
    }
    props.ref?.(node);
  }} />;
}

const imageMatrixShader = `
fn apply_row(color: vec4<f32>, index: u32) -> f32 {
    let row = vec4(effect_uniform(index), effect_uniform(index + 1u),
        effect_uniform(index + 2u), effect_uniform(index + 3u));
    return dot(color, row) + effect_uniform(index + 4u);
}

fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let color = effect_sample(0u, local_pos / max(size, vec2(1.0)));
    return vec4(apply_row(color, 0u), apply_row(color, 5u),
        apply_row(color, 10u), apply_row(color, 15u));
}`;

export interface ImageColorFilterProps extends Omit<ViewProps, "children"> {
  source: string;
  matrix: ColorMatrix;
}

/** Draws an image with a linear RGBA color matrix. */
export function ImageColorFilter(props: ImageColorFilterProps): NodeHandle {
  const program = createShaderProgram(imageMatrixShader);
  return <ShaderEffect {...props} program={program} textures={[props.source]}
    uniforms={packed(props.matrix.flat())} />;
}

const edgeBorderShader = `
fn effect_main(local_pos: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let widths = max(vec4(effect_uniform(0u), effect_uniform(1u), effect_uniform(2u),
        effect_uniform(3u)), vec4(0.0));
    let edges = vec4(local_pos.y, size.x - local_pos.x, size.y - local_pos.y, local_pos.x);
    let normalized = select(vec4(1e6), edges / max(widths, vec4(0.001)), widths > vec4(0.0));
    var side = 0u;
    if (normalized.y < normalized[side]) { side = 1u; }
    if (normalized.z < normalized[side]) { side = 2u; }
    if (normalized.w < normalized[side]) { side = 3u; }
    let stroke = (1.0 - smoothstep(widths[side] - 0.5, widths[side] + 0.5, edges[side]))
        * select(0.0, 1.0, widths[side] > 0.0);
    let solid = effect_uniform(20u);
    let gap = effect_uniform(21u);
    let position = select(local_pos.x, local_pos.y, side % 2u == 1u);
    let phase = fract(position / max(solid + gap, 1.0)) * (solid + gap);
    let dash = select(1.0, 1.0 - smoothstep(solid - 0.5, solid + 0.5, phase), solid > 0.0 && gap > 0.0);
    let index = 4u + side * 4u;
    return vec4(effect_uniform(index), effect_uniform(index + 1u), effect_uniform(index + 2u),
        effect_uniform(index + 3u) * stroke * dash);
}`;

export interface EdgeBorderProps extends Omit<ViewProps, "children"> {
  widths: readonly [top: number, right: number, bottom: number, left: number];
  colors: readonly [top: MaterialColor, right: MaterialColor, bottom: MaterialColor, left: MaterialColor];
  dash?: readonly [length: number, gap: number];
}

/** Draws an overlay with independent widths and colors for its four edges. */
export function EdgeBorder(props: EdgeBorderProps): NodeHandle {
  const program = createShaderProgram(edgeBorderShader);
  return <ShaderEffect {...props} program={program}
    uniforms={packed([...props.widths, ...props.colors.flat(), ...(props.dash ?? [0, 0])])} />;
}

const highlightBorderShader = `
fn effect_main(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let top = p.y;
    let right = size.x - p.x;
    let bottom = size.y - p.y;
    let left = p.x;
    let edge = min(min(top, right), min(bottom, left));
    var along = p.x;
    if (edge == right) {
        along = size.x + p.y;
    } else if (edge == bottom) {
        along = size.x + size.y + right;
    } else if (edge == left) {
        along = 2.0 * size.x + size.y + bottom;
    }
    let perimeter = max(1.0, 2.0 * (size.x + size.y));
    let phase = fract(along / perimeter - effect_time() * effect_uniform(1u));
    let glow = 1.0 - smoothstep(0.02, 0.19, min(phase, 1.0 - phase));
    let width = effect_uniform(0u);
    let stroke = 1.0 - smoothstep(max(0.0, width - 0.5), width + 0.5, edge);
    let color = vec3(effect_uniform(2u), effect_uniform(3u), effect_uniform(4u));
    return vec4(color, effect_uniform(5u) * stroke * mix(0.2, 1.0, glow));
}`;

export interface HighlightBorderProps extends Omit<ViewProps, "children"> {
  width?: number;
  periodMs?: number;
  color?: MaterialColor;
  animated?: boolean;
}

/** Draws a border highlight using the compositor clock. */
export function HighlightBorder(props: HighlightBorderProps): NodeHandle {
  const animated = createShaderProgram(highlightBorderShader, { animated: true });
  const still = createShaderProgram(highlightBorderShader);
  return (
    <ShaderEffect {...props}
      program={props.animated !== false && !scene.reduceMotion() ? animated : still}
      uniforms={packed([
        Math.max(0.5, props.width ?? 1.5),
        1000 / Math.max(250, props.periodMs ?? 1600),
        ...(props.color ?? [1, 1, 1, 0.6]),
      ])} />
  );
}
