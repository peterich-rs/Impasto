import { createSignal, onCleanup } from "solid-js";
import { onLayout, scene, type NodeHandle } from "@impasto/solid-renderer";
import { createScrollController, useScrollThreshold } from "@impasto/solid-ui";

// Refresh a section slightly before it enters the viewport.
const margin = 48;

export function trackStockSections() {
  const controller = createScrollController();
  return {
    controller,
    section() {
      let node: NodeHandle | null = null;
      const [bounds, setBounds] = createSignal<{ enter: number; exit: number } | null>(null);
      onCleanup(onLayout(() => {
        const viewport = controller.node();
        if (node === null || viewport === 0) return;
        const view = scene.rect(viewport);
        const box = scene.rect(node);
        const top = box.y - view.y;
        const enter = Math.max(0, top - view.h - margin);
        const exit = top + box.h + margin;
        setBounds((previous) =>
          previous?.enter === enter && previous.exit === exit ? previous : { enter, exit });
      }));
      const entered = useScrollThreshold(controller, {
        get enabled() { return bounds() !== null; },
        get value() { return bounds()?.enter ?? 0; },
      });
      const exited = useScrollThreshold(controller, {
        from: "start",
        get enabled() { return bounds() !== null; },
        get value() { return bounds()?.exit ?? 0; },
      });
      return {
        visible: () => bounds() === null || (entered() && !exited()),
        ref(next: NodeHandle | null) {
          node = next;
          setBounds(null);
        },
      };
    },
  };
}
