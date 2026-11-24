import { createRoot, createTrackedEffect, onCleanup } from "solid-js";
import { scene, type NodeHandle, type ScrollChannel } from "@impasto/solid-renderer";
import type { ScrollController } from "./ScrollController.ts";

export type ScrollCurve = Readonly<{
  input: readonly number[];
  output: readonly number[];
  extrapolate: "clamp" | "extend";
}>;
export type ScrollColorCurve = Readonly<{
  input: readonly number[];
  output: readonly string[];
  extrapolate: "clamp";
}>;

export type ScrollStyleBindings = {
  source?: ScrollChannel;
  translateX?: ScrollCurve;
  translateY?: ScrollCurve;
  scale?: ScrollCurve;
  scaleX?: ScrollCurve;
  scaleY?: ScrollCurve;
  rotate?: ScrollCurve;
  opacity?: ScrollCurve;
  blur?: ScrollCurve;
  backdropBlur?: ScrollCurve;
  backgroundColor?: ScrollColorCurve;
  shaderUniform?: Readonly<Record<number, ScrollCurve>>;
};

/** Describes a scalar interpolation in scene points. Rotation is in radians. */
export function interpolate(
  input: readonly number[],
  output: readonly number[],
  extrapolate: ScrollCurve["extrapolate"] = "clamp",
): ScrollCurve {
  validateStops(input, output);
  if (!output.every(Number.isFinite)) throw new RangeError("Scroll outputs must be finite");
  return { input: [...input], output: [...output], extrapolate };
}

/** Interpolates CSS colors in premultiplied linear color space. */
export function interpolateColor(
  input: readonly number[],
  output: readonly string[],
): ScrollColorCurve {
  validateStops(input, output);
  return { input: [...input], output: [...output], extrapolate: "clamp" };
}

function validateStops(input: readonly number[], output: readonly unknown[]): void {
  if (input.length < 2 || input.length !== output.length || input.some((stop, index) =>
    !Number.isFinite(Math.fround(stop)) || (index > 0 && Math.fround(stop) <= Math.fround(input[index - 1]!)),
  )) throw new RangeError("Scroll inputs must increase and match the number of outputs");
}

/** Applies curves to a node without subscribing to continuous JavaScript updates. */
export function bindScrollStyle(
  target: NodeHandle,
  controller: ScrollController,
  bindings: ScrollStyleBindings,
): () => void {
  return createRoot((dispose) => {
    createTrackedEffect(() => {
      const source = controller.node();
      if (source === 0 || target === 0) return;
      const { source: channel = "scrollY", shaderUniform, ...style } = bindings;
      const bind = (property: string, curve?: ScrollCurve | ScrollColorCurve) => {
        if (!curve) return;
        const id = scene.bindScrollProp(source, channel, target, property, curve.input, curve.output, curve.extrapolate);
        onCleanup(() => scene.unbindScrollProp(id));
      };
      for (const [property, curve] of Object.entries(style)) bind(property, curve);
      for (const [index, curve] of Object.entries(shaderUniform ?? {})) bind(`shaderUniform:${index}`, curve);
    });
    return dispose;
  });
}
