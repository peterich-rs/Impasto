import * as scene from "scene";
import {
  addEventListener,
  getWindowState,
  setRestorationState,
  setWindowState,
} from "@impasto/app-state/native";

scene.createElement("text", { value: "Window state", style: { fontSize: 16, color: "#ffffff" } });
const refresh = () => setRestorationState("shared", getWindowState("selection"));
refresh();
addEventListener("windowState", refresh);
scene.setBackEnabled(true);
scene.onBackGesture((phase) => {
  if (phase === "commit") {
    setWindowState("selection", String(Number(getWindowState("selection")) + 1));
  }
});
scene.onWindowMetrics(({ width }) => {
  if (width > 0) setRestorationState("width", String(width));
});
