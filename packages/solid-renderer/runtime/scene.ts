import { scene as native } from "scene";
import type { SceneModule } from "../src/scene.ts";
import type { Node } from "../src/tree.ts";

export const scene = native as unknown as SceneModule & {
  createElement(tag: string, props?: Record<string, unknown>): Node;
  createTextNode(value: string | number): Node;
  insert(parent: Node, children: unknown, marker?: Node | null): void;
  insertNode(parent: Node, node: Node, anchor?: Node): void;
  setProp(node: Node, name: string, value: unknown): void;
};
