import { counterScene, formatScene } from "../../frameworks/solid-renderer/src/scene.ts";

const scene = counterScene();
console.log(formatScene(scene));
console.log(`encoded ${scene.encode().byteLength} bytes`);
