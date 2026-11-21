import * as scene from "scene";
import {
  addEventListener,
  getWindowState,
  setRestorationState,
  setWindowState,
} from "@impasto/app-state";

scene.text("Window state", 16, [255, 255, 255, 255]);
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
