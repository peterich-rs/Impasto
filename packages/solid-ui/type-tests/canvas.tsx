import { Canvas, type CanvasController } from "../src/index.tsx";

const controller = (value: CanvasController | null) => {
  value?.invalidate();
};
<Canvas
  style={{ width: 40, height: 20 }}
  draw={(ctx, size) => {
    ctx.fillRect(0, 0, size.width, size.height);
  }}
  controller={controller}
/>;

// @ts-expect-error canvas does not take children
<Canvas style={{ width: 40, height: 20 }} draw={() => {}}>
  child
</Canvas>;

// @ts-expect-error draw is required
<Canvas style={{ width: 40, height: 20 }} />;
