import * as scene from "scene";

const column = scene.column(0);
const field = scene.text("draft", 18, [255, 255, 255, 255]);
scene.setPadding(column, 24);
scene.append(column, field);
scene.setFocusable(field, true);

function publish(text, start, end) {
  scene.setSemantics(field, 6, "Inspection note", text, -1, start, end, 0, 1, 0, "");
}

scene.onTextReplaced(field, publish);
publish("draft", 5, 5);
scene.focus(field);
