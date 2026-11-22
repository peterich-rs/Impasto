import * as scene from "scene";

const column = scene.createElement("view");
const field = scene.createElement("text", {
  value: "draft",
  style: { fontSize: 18, color: "#ffffff" },
});
scene.patchStyle(column, { padding: 24 });
scene.append(column, field);
scene.setFocusable(field, true);

function publish(text, start, end) {
  scene.setSemantics(field, 6, "Inspection note", text, -1, start, end, 0, 1, 0, "");
}

scene.onTextReplaced(field, publish);
publish("draft", 5, 5);
scene.focus(field);
