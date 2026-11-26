import * as scene from "scene";
import { setRestorationState } from "@impasto/app-state/native";

const column = scene.createElement("view");
const field = scene.createElement("view");
scene.patchStyle(field, { width: 128, height: 48 });
scene.append(column, field);
const publish = (value) => scene.setSemantics(
  field, 6, "Test password", undefined, -1, -1, -1, 0, 1, 0, "",
  { password: true, autofill: { hint: "password", value } },
);
publish("");
scene.setFocusable(field, true);
scene.focus(field);
scene.onTextReplaced(field, (value) => {
  publish(value);
  setRestorationState("filled", value === "Android fill 😀" ? "yes" : "unexpected");
});
