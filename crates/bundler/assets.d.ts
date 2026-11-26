declare module "*.wgsl" {
  const source: string;
  export const uniformIndex: Readonly<Record<string, number>>;
  export function packUniforms(values: Record<string, number | readonly number[]>): Uint8Array;
  export default source;
}

declare module "*.png" {
  const source: import("@impasto/solid-renderer").ImageAsset;
  export default source;
}
declare module "*.jpg" {
  const source: import("@impasto/solid-renderer").ImageAsset;
  export default source;
}
declare module "*.jpeg" {
  const source: import("@impasto/solid-renderer").ImageAsset;
  export default source;
}
declare module "*.webp" {
  const source: import("@impasto/solid-renderer").ImageAsset;
  export default source;
}
declare module "*.ttf" {
  const font: { family: string; weight: number };
  export default font;
}
declare module "*.otf" {
  const font: { family: string; weight: number };
  export default font;
}
declare module "*.svg" {
  const icon: import("@impasto/solid-renderer").VectorAsset;
  export default icon;
}
