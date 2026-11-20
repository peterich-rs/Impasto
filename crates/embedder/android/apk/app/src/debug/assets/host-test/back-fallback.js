import * as scene from "scene";

scene.text("Back fallback", 18, [255, 255, 255, 255]);
scene.onBackGesture((phase) => phase !== "commit");
scene.setBackEnabled(true);
