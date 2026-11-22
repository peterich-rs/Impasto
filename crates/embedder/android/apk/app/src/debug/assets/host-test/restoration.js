import * as scene from "scene";
import { getRestorationState, setRestorationState } from "@impasto/app-state";
import { Linking } from "@impasto/linking";

const summary = scene.createElement("text", { value: "Android task", style: { fontSize: 18, color: "#ffffff" } });
setRestorationState("starts", String(Number(getRestorationState("starts")) + 1));
setRestorationState("initialUrl", Linking.getInitialURL());
scene.onBackGesture((phase, value) => {
  setRestorationState("gesture", phase === "progress" ? `${phase}:${value}` : phase);
  if (phase === "commit") scene.setBackEnabled(false);
});
scene.setBackEnabled(true);
scene.onWindowMetrics((metrics) => {
  if (metrics.width > 0 && metrics.refreshRate > 0) {
    setRestorationState("refreshRate", String(metrics.refreshRate));
    const features = metrics.displayFeatures?.map((feature) => feature.posture ?? feature.type);
    scene.setText(summary, `Android task ${Math.round(metrics.refreshRate)} Hz ${features?.join(",") || "no features"}`);
  }
});
