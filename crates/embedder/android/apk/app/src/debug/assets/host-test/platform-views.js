import * as scene from "scene";

const root = scene.column(0);
const editor = scene.view();
scene.setSize(editor, 128, 48);
scene.setPlatformView(editor, "edit");
scene.append(root, editor);
const media = scene.image("external:camera", 64, 64, 0, false);
scene.append(root, media);
