import { ExternalTexture } from "@impasto/solid-ui";
import type { CameraSession } from "@impasto/camera/native";

export { openCamera, type CameraSession, type Facing } from "@impasto/camera/native";

export function CameraPreview(props: { session: CameraSession }) {
  return (
    <ExternalTexture
      name={props.session.textureKey()}
      style={{ width: "100%", height: "100%" }}
    />
  );
}
