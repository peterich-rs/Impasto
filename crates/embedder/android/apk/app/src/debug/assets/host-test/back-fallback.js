import * as scene from "scene";

scene.createElement("text", { value: "Back fallback", style: { fontSize: 18, color: "#ffffff" } });
scene.onBackGesture((phase) => phase !== "commit");
scene.setBackEnabled(true);
