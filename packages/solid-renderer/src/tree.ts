/** Opaque generational handle allocated by the engine tree. Zero means absent. */
export type Node = number;

/** Renderer hooks used by leaf components. Every operation calls `scene`. */
export interface SceneOptions {
  styles: Pick<
    import("./scene.ts").SceneModule,
    "createStyleSource" | "setStyleSource" | "removeStyleSource"
  >;
  createElement(tag: string, props?: Record<string, unknown>): Node;
  createTextNode(value: string | number): Node;
  replaceText(node: Node, value: string | number): void;
  isTextNode(node: Node): boolean;
  setProperty(
    node: Node,
    name: string,
    value: unknown,
    previous?: unknown,
  ): void;
  insertNode(parent: Node, node: Node, anchor?: Node): void;
  removeNode(parent: Node, node: Node): void;
  getParentNode(node: Node): Node | undefined;
  getFirstChild(node: Node): Node | undefined;
  getNextSibling(node: Node): Node | undefined;
}
