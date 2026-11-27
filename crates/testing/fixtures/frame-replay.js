import * as scene from "scene";

const root = scene.createElement("view", {
  style: { width: "100%", height: "100%", backgroundColor: "#101722" },
});
const text = (value, size = 16) => scene.createElement("text", {
  value, style: { fontSize: size, color: "#FFFFFF" },
});
function button(label, action) {
  const node = scene.createElement("view", { style: { height: 48, flexShrink: 0 } });
  scene.append(node, text(label));
  scene.setSemantics(node, 1, label);
  scene.onClick(node, action);
  return node;
}
const home = scene.createElement("view", { style: { flex: 1, minHeight: 0 } });
const detail = scene.createElement("view", { style: { flex: 1, display: "none" } });
const dialog = scene.createElement("view", { style: { display: "none" } });
const title = text("Frame replay", 34);
scene.append(home, title);
let visits = 0;
scene.append(home, button("Open detail", () => {
  scene.patchStyle(home, { display: "none" });
  scene.patchStyle(detail, { display: "flex" });
  Promise.resolve().then(() => scene.setText(detailTitle, `Detail ${++visits}`));
}));
const scroll = scene.createElement("scroll", { axis: "vertical", style: { flex: 1, minHeight: 0 } });
scene.append(home, scroll);
const rows = scene.createElement("view", { style: { flexShrink: 0 } });
scene.append(scroll, rows);
const labels = ["你好", "こんにちは", "안녕하세요", "Grüße", "مرحبا", "नमस्ते", "👩‍🚀"];
const tick = text("Version 0");
scene.append(home, tick);
let data = 137;
let version = 0;
setInterval(() => {
  data = (Math.imul(data, 1664525) + 1013904223) >>> 0;
  scene.setText(tick, `Version ${++version} · ${data % 10000}`);
}, 200);
for (let index = 0; index < 120; index++) {
  const row = text(`Row ${index} · ${labels[index % labels.length]}`);
  scene.patchStyle(row, { height: 36, flexShrink: 0 });
  scene.append(rows, row);
}
const detailTitle = text("Detail", 42);
scene.append(detail, detailTitle);
scene.append(detail, button("Back", () => {
  scene.blur();
  scene.patchStyle(detail, { display: "none" });
  scene.patchStyle(home, { display: "flex" });
}));
scene.append(detail, button("Show dialog", () => scene.patchStyle(dialog, { display: "flex" })));
const editor = scene.createElement("view", { style: { height: 48, flexShrink: 0 } });
const editorText = text("Edit text");
scene.append(editor, editorText);
scene.setFocusable(editor, true);
let value = "";
function publishEditor(anchor = value.length, caret = anchor) {
  scene.setText(editorText, value || "Edit text");
  scene.setSemantics(editor, 6, "Replay editor", value, undefined, anchor, caret, 0, 1, 0);
}
function publishIme(anchor = value.length, caret = anchor, from = -1, to = -1) {
  scene.setImeState(value, anchor, caret, from, to, 0, 0, 1, 24);
}
publishEditor();
scene.onClick(editor, () => scene.focus(editor));
scene.onFocus(editor, () => publishIme());
scene.onBlur(editor, () => scene.setImeState(null));
scene.onTextReplaced(editor, (next, anchor, caret, from, to) => {
  value = next;
  publishEditor(anchor, caret);
  publishIme(anchor, caret, from, to);
});
scene.append(detail, editor);
const canvas = scene.createElement("view", { style: { height: 80 } });
scene.bindCanvas(canvas, ctx => {
  ctx.fillStyle = "#24A884";
  ctx.fillRect(4, 4, 76, 48);
  ctx.fillStyle = "#FFFFFF";
  ctx.fillText("Canvas", 16, 28);
}, null, true);
scene.append(detail, canvas);
scene.append(dialog, button("Close dialog", () => scene.patchStyle(dialog, { display: "none" })));
scene.append(detail, dialog);
scene.append(root, home);
scene.append(root, detail);
