import { createSignal } from "solid-js";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import { Canvas, type CanvasController } from "@impasto/solid-ui";
import { colorBytes } from "../colors.ts";
import { loc } from "../copy.ts";

const INK = "#1F2430";
const INK_COLOR = colorBytes(INK);
const PANEL = "#FFFFFF";
const HAIR = "#E8E8EC";
const UP = "#EB3E3E";
const DOWN = "#22B05F";

/** Capability page for the screen canvas. Charts on the stock page stay separate. */
export function CanvasPage(): NodeHandle {
  const page = scene.createElement("view", {
    style: { gap: 12, padding: 16, backgroundColor: PANEL },
  });
  scene.patchStyle(page, { flex: 1 });
  const title = scene.createElement("text", {
    value: loc("绘图", "Canvas"),
    style: { fontSize: 20, fontWeight: 600, color: INK_COLOR },
  });
  scene.append(page, title);
  scene.append(page, block(loc("折线与曲线", "Line and curve"), (ctx, size) => {
    ctx.strokeStyle = INK;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(8, size.height - 12);
    ctx.lineTo(size.width * 0.45, 16);
    ctx.quadraticCurveTo(size.width * 0.7, size.height - 8, size.width - 8, 20);
    ctx.stroke();
  }));
  scene.append(page, block(loc("变换", "Transform"), (ctx, size) => {
    ctx.translate(size.width / 2, size.height / 2);
    ctx.rotate(Math.PI / 8);
    ctx.fillStyle = UP;
    ctx.fillRect(-28, -14, 56, 28);
  }));
  scene.append(page, block(loc("裁剪", "Clip"), (ctx, size) => {
    ctx.beginPath();
    ctx.arc(size.width / 2, size.height / 2, 22, 0, Math.PI * 2);
    ctx.clip();
    ctx.fillStyle = DOWN;
    ctx.fillRect(0, 0, size.width, size.height);
  }));
  scene.append(page, block(loc("清除不擦掉背景", "clearRect keeps the background"), (ctx, size) => {
    ctx.fillStyle = HAIR;
    ctx.fillRect(0, 0, size.width, size.height);
    ctx.clearRect(16, 12, size.width - 32, size.height - 24);
  }));
  scene.append(page, block(loc("空白", "Empty"), () => {}));
  const [note, setNote] = createSignal(loc("旧画面还在", "Previous picture stays"));
  const [fail, setFail] = createSignal(false);
  let picture: CanvasController | null = null;
  const errorLabel = scene.createElement("text", {
    value: note(),
    style: { fontSize: 12, color: INK_COLOR },
  });
  const errorBlock = block(
    loc("失败时保留上一帧", "A failed draw keeps the last picture"),
    (ctx, size) => {
      if (fail()) throw new Error("draw failed");
      ctx.fillStyle = UP;
      ctx.fillRect(8, 8, size.width - 16, size.height - 16);
    },
    (next) => {
      picture = next;
    },
    (error) => {
      setNote(error.message);
      scene.setText(errorLabel, error.message);
    },
  );
  scene.append(page, errorBlock);
  scene.append(page, errorLabel);
  const retry = scene.createElement("view", {
    style: { paddingTop: 8, paddingBottom: 8 },
  });
  scene.onClick(retry, () => {
    setFail(true);
    picture?.invalidate();
  });
  const retryLabel = scene.createElement("text", {
    value: loc("让下一次绘制失败", "Fail the next draw"),
    style: { fontSize: 14, color: UP },
  });
  scene.append(retry, retryLabel);
  scene.append(page, retry);
  return page;
}

function block(
  title: string,
  draw: CanvasPageDraw,
  controller?: (controller: CanvasController | null) => void,
  onDrawError?: (error: { message: string }) => void,
): NodeHandle {
  const column = scene.createElement("view", { style: { gap: 6 } });
  const label = scene.createElement("text", {
    value: title,
    style: { fontSize: 13, color: INK_COLOR },
  });
  scene.append(column, label);
  scene.append(
    column,
    Canvas({
      style: {
        width: 280,
        height: 72,
        backgroundColor: PANEL,
        borderRadius: 8,
        outlineWidth: 1,
        outlineColor: HAIR,
      },
      draw,
      controller,
      onDrawError,
    }),
  );
  return column;
}

type CanvasPageDraw = NonNullable<Parameters<typeof Canvas>[0]["draw"]>;
