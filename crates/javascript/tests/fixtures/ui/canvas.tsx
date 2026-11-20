import { createRoot } from "solid-js";
import { Canvas, Column } from "@impasto/solid-ui";

createRoot(() => (
  <Column>
    <Canvas
      style={{ width: 120, height: 80 }}
      draw={(ctx) => {
        ctx.fillStyle = "#112233";
        ctx.fillRect(0, 0, 12, 8);
      }}
    />
    <Canvas
      style={{ width: 120, height: 80 }}
      draw={async (ctx) => {
        ctx.fillRect(0, 0, 12, 8);
        await Promise.resolve();
      }}
    />
  </Column>
));
