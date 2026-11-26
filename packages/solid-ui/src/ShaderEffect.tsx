import { View as HostView } from "@impasto/solid-renderer/host";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { createRenderEffect, onCleanup } from "solid-js";
import { bindBox } from "./element.tsx";
import type { ViewProps } from "./props.ts";
import { View } from "./View.tsx";

declare const shaderProgram: unique symbol;

/** A fragment program registered in the current scene session. */
export type ShaderProgram = number & { readonly [shaderProgram]: true };

/** Registers WGSL with an `effect_main(local_pos, size)` entry point. */
export function createShaderProgram(
  source: string,
  options?: { animated?: boolean },
): ShaderProgram {
  return scene.registerShaderProgram(source, options?.animated) as ShaderProgram;
}

/** Samples the pixels which precede the effect in paint order. */
export const shaderBackdropTexture = "backdrop:";

/** Drives one effect parameter from its current value on the compositor clock. */
export interface ShaderEffectController {
  animateUniform(
    index: number,
    target: number,
    durationMs: number,
    easing?: "linear" | "ease-in-out",
  ): void;
}

export interface ShaderEffectProps extends Omit<ViewProps, "children"> {
  program: ShaderProgram | null;
  /** Up to 32 packed little-endian f32 values read by `effect_uniform(index)`. */
  uniforms?: Uint8Array;
  /** Up to four image sources read by `effect_sample(index, uv)`. */
  textures?: readonly string[];
  controller?: (controller: ShaderEffectController | null) => void;
}

/** Draws an effect inside a layout box and its clip. */
export function ShaderEffect(props: ShaderEffectProps): NodeHandle {
  let surface: NodeHandle | null = null;
  onCleanup(() => {
    surface = null;
    props.controller?.(null);
  });
  return (
    <View {...props}>
      <HostView ref={(node) => {
        if (node === null) return;
        surface = node;
        bindBox(node, {}, { width: "100%", height: "100%" });
        createRenderEffect(
          () => ({ program: props.program, uniforms: props.uniforms, textures: props.textures }),
          ({ program, uniforms, textures }) => scene.setShaderEffect(node, program, uniforms, textures),
        );
        props.controller?.({
          animateUniform(index, target, durationMs, easing) {
            if (surface !== null) scene.animateShaderUniform(surface, index, target, durationMs, easing);
          },
        });
      }} />
    </View>
  );
}
