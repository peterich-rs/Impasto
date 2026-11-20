import * as scene from "scene";

const root = scene.column(0);
const canvas = scene.sized(128, 96);
scene.append(root, canvas);
scene.bindCanvas(canvas, (ctx) => {
  ctx.strokeStyle = "#20e090";
  ctx.lineWidth = 4;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(8, 72);
  ctx.quadraticCurveTo(24, 4, 48, 32);
  ctx.quadraticCurveTo(72, 66, 112, 16);
  ctx.stroke();
});
