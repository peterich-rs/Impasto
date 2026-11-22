import { boot } from "@impasto/solid-renderer";
import { ExternalTexture, PlatformView, Role, View } from "@impasto/solid-ui";

function Fixture() {
  return (
    <view style={{ width: "100%", height: "100%", backgroundColor: "#15191f" }}>
      <View role={Role.Button} accessibilityLabel="框架前项"
        style={{ position: "absolute", top: 20, left: 12, width: 240, height: 120, backgroundColor: "#d00000", zIndex: 0 }} />
      <PlatformView kind="green" clipPath="M 22 0 H 192 V 70 H 0 V 18 H 22 Z"
        style={{ position: "absolute", top: 36, left: 28, width: 192, height: 70, zIndex: 1 }} />
      <View role={Role.Button} accessibilityLabel="框架后项"
        style={{ position: "absolute", top: 30, left: 88, width: 102, height: 80, backgroundColor: "#0000dd", zIndex: 2 }} />
      <PlatformView kind="orange"
        style={{ position: "absolute", top: 64, left: 132, width: 110, height: 64, zIndex: 3 }} />
      <view style={{ position: "absolute", top: 80, left: 168, width: 62, height: 40, backgroundColor: "#ff00ff", zIndex: 4 }} />
      <ExternalTexture name="bgra"
        style={{ position: "absolute", top: 160, left: 12, width: 96, height: 64, zIndex: 5 }} />
      <ExternalTexture name="nv12"
        style={{ position: "absolute", top: 160, left: 120, width: 96, height: 64, zIndex: 5 }} />
    </view>
  );
}

boot(Fixture);
