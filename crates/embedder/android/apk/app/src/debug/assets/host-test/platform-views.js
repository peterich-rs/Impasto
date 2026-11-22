import * as scene from "scene";

const root = scene.createElement("view");
const editor = scene.createElement("view");
scene.patchStyle(editor, { width: 128, height: 48 });
scene.setPlatformView(editor, "edit");
scene.append(root, editor);
const media = scene.createElement("image", {
  source: "external:camera",
  intrinsicSize: { width: 64, height: 64 },
});
scene.append(root, media);
