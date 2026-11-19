import { createSignal, onCleanup } from "solid-js";
import { onLayout, scene, type NodeHandle } from "@impasto/solid-renderer";

// Warm sections just before they enter the viewport during a vertical drag.
const margin = 48;

export function trackStockSections() {
  let viewport: NodeHandle | null = null;
  let height = 0;
  const sections: { node: NodeHandle | null; top: number; bottom: number; shown: boolean; setVisible: (visible: boolean) => void }[] = [];
  const update = (y: number) => {
    for (const section of sections) {
      if (section.node === null) continue;
      const shown = section.bottom > y - margin && section.top < y + height + margin;
      if (section.shown !== shown) {
        section.shown = shown;
        section.setVisible(shown);
      }
    }
  };
  onCleanup(onLayout(() => {
    if (viewport === null) return;
    const view = scene.rect(viewport);
    height = view.h;
    for (const section of sections) {
      if (section.node === null) continue;
      const box = scene.rect(section.node);
      section.top = box.y - view.y;
      section.bottom = section.top + box.h;
    }
    // Edge bounce reveals empty space. The clamped window covers its in-flow sections.
    update(scene.scrollOffset(viewport).y);
  }));
  return {
    viewportRef(node: NodeHandle | null) {
      viewport = node;
      if (node !== null) {
        scene.onScroll(node, (_x, y) => update(y));
      }
    },
    section() {
      const [visible, setVisible] = createSignal(true);
      const section = { node: null as NodeHandle | null, top: 0, bottom: Infinity, shown: true, setVisible };
      sections.push(section);
      return {
        visible,
        ref(node: NodeHandle | null) {
          section.node = node;
          if (node !== null) {
            section.top = 0;
            section.bottom = Infinity;
            section.shown = true;
            setVisible(true);
          }
        },
      };
    },
  };
}
