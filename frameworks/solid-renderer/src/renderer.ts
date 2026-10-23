import { parseColor } from "./colors.ts";
import { KIND_IMAGE, KIND_LAYER, KIND_RECT, KIND_TEXT, type Rgba, type Scene, type SceneNode } from "./scene.ts";

/** Structural match for `@solidjs/universal` RendererOptions. Solid is not imported here. */
export interface SceneRendererOptions {
  createElement(tag: string, staticProps?: Record<string, unknown>): SceneNode;
  createTextNode(value: string): SceneNode;
  replaceText(textNode: SceneNode, value: string): void;
  isTextNode(node: SceneNode): boolean;
  setProperty(node: SceneNode, name: string, value: unknown, prev?: unknown): void;
  insertNode(parent: SceneNode, node: SceneNode, anchor?: SceneNode): void;
  removeNode(parent: SceneNode, node: SceneNode): void;
  getParentNode(node: SceneNode): SceneNode | undefined;
  getFirstChild(node: SceneNode): SceneNode | undefined;
  getNextSibling(node: SceneNode): SceneNode | undefined;
}

const clear: Rgba = { r: 0, g: 0, b: 0, a: 0 };

function applyProps(node: SceneNode, props: Record<string, unknown> | undefined) {
  if (props === undefined) {
    return;
  }
  for (const [name, value] of Object.entries(props)) {
    setProp(node, name, value);
  }
}

function setProp(node: SceneNode, name: string, value: unknown) {
  switch (name) {
    case "x":
    case "y":
    case "w":
    case "h":
      if (typeof value !== "number") {
        throw new Error(`${name} must be a number`);
      }
      node[name] = value;
      return;
    case "color": {
      const color = parseColor(value);
      node.r = color.r;
      node.g = color.g;
      node.b = color.b;
      node.a = color.a;
      return;
    }
    case "text":
    case "source":
      if (typeof value !== "string") {
        throw new Error(`${name} must be a string`);
      }
      node.text = value;
      return;
    default:
      throw new Error(`unknown prop ${name}`);
  }
}

function unlink(scene: Scene, node: SceneNode) {
  if (node.parentId === 0) {
    return;
  }
  const parent = scene.node(node.parentId);
  parent.children = parent.children.filter((child) => child !== node);
  node.parentId = 0;
}

export function sceneRendererOptions(scene: Scene): SceneRendererOptions {
  return {
    createElement(tag, staticProps) {
      let node: SceneNode;
      switch (tag) {
        case "layer":
          node = scene.node(scene.layer(0));
          break;
        case "rect":
          node = scene.node(scene.rect(0, 0, 0, 0, 0, clear));
          break;
        case "text":
          node = scene.node(scene.text(0, 0, 0, "", clear));
          break;
        case "image":
          node = scene.node(scene.image(0, 0, 0, 0, 0, ""));
          break;
        default:
          throw new Error(`unknown tag ${tag}`);
      }
      applyProps(node, staticProps);
      return node;
    },
    createTextNode(value) {
      return scene.node(scene.text(0, 0, 0, value, clear));
    },
    replaceText(textNode, value) {
      textNode.text = value;
    },
    isTextNode(node) {
      return node.kind === KIND_TEXT;
    },
    setProperty(node, name, value) {
      setProp(node, name, value);
    },
    insertNode(parent, node, anchor) {
      if (node.parentId !== 0) {
        unlink(scene, node);
      }
      node.parentId = parent.id;
      if (anchor === undefined) {
        parent.children.push(node);
        return;
      }
      const index = parent.children.indexOf(anchor);
      if (index < 0) {
        throw new Error("anchor is not a child of parent");
      }
      parent.children.splice(index, 0, node);
    },
    removeNode(parent, node) {
      if (node.parentId !== parent.id) {
        throw new Error("node is not a child of parent");
      }
      unlink(scene, node);
    },
    getParentNode(node) {
      return node.parentId === 0 ? undefined : scene.node(node.parentId);
    },
    getFirstChild(node) {
      return node.children[0];
    },
    getNextSibling(node) {
      const parent = node.parentId === 0 ? undefined : scene.node(node.parentId);
      if (parent === undefined) {
        return undefined;
      }
      const index = parent.children.indexOf(node);
      return parent.children[index + 1];
    },
  };
}

export const rendererKind = { KIND_LAYER, KIND_RECT, KIND_TEXT, KIND_IMAGE };
