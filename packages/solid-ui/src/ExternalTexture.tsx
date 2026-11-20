import { Image, type ImageProps, type NodeHandle } from "@impasto/solid-renderer";

export type ExternalTextureProps = Omit<ImageProps, "source" | "required"> & {
  name: string;
};

/** Displays frames published by an external producer registered on the native host. */
export function ExternalTexture(props: ExternalTextureProps): NodeHandle {
  return <Image {...props} source={`external:${props.name}`} />;
}
