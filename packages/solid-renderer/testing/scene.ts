import type { SceneModule, TextSpanData } from "../src/scene.ts";
import type { Node, SceneOptions } from "../src/tree.ts";

export type RetainedCounts = { nodes: number };

/** Node-only bookkeeping for testing the renderer against a mock scene. */
export function testScene(host: SceneModule): SceneOptions & {
  forget(node: Node): void;
  retainedCounts(): RetainedCounts;
} {
  const nodes = new Set<Node>();

  function track(node: Node): Node {
    nodes.add(node);
    return node;
  }

  function defined(node: Node): Node | undefined {
    return node === 0 ? undefined : track(node);
  }

  function forget(node: Node, seen = new Set<Node>()): void {
    if (node === 0 || seen.has(node)) return;
    seen.add(node);
    const listed = new Set<Node>();
    let child = host.firstChild(node);
    while (child !== 0 && !seen.has(child) && !listed.has(child)) {
      listed.add(child);
      child = host.nextSibling(child);
    }
    for (const id of listed) forget(id, seen);
    nodes.delete(node);
  }

  function setProperty(node: Node, name: string, value: unknown): void {
    switch (name) {
      case "style":
        host.setStyle(node, value as Parameters<SceneModule["setStyle"]>[1]);
        return;
      case "class":
        host.setClass(node, String(value ?? ""));
        return;
      case "classList":
        host.setClass(node, Object.entries((value ?? {}) as Record<string, boolean>)
          .filter(([, enabled]) => enabled).map(([name]) => name).join(" "));
        return;
      case "value":
        host.setText(node, String(value ?? ""));
        return;
      case "runs":
        host.setTextRuns(node, (value ?? []) as TextSpanData[]);
        return;
      case "axis":
        host.setProp?.(node, name, value);
        return;
      case "focusable":
        host.setFocusable(node, Boolean(value));
        return;
      case "recognizers":
        host.setRecognizers?.(node, value as Parameters<NonNullable<SceneModule["setRecognizers"]>>[1]);
        return;
      case "children":
      case "ref":
        return;
      default: {
        const listener = Reflect.get(host, name);
        if (name.startsWith("on") && typeof listener === "function") {
          listener(node, value);
          return;
        }
        throw new Error(`unknown prop ${name}`);
      }
    }
  }

  return {
    styles: host,
    createElement(tag, props) {
      const node = track(host.createElement(tag, props));
      for (const [name, value] of Object.entries(props ?? {})) {
        if (name === "style" || (tag === "text" && name === "value")
          || (tag === "image" && ["source", "intrinsicSize", "required"].includes(name))
          || (tag === "scroll" && name === "axis")) continue;
        setProperty(node, name, value);
      }
      return node;
    },
    createTextNode: (value) => track(host.createTextNode(value)),
    replaceText: (node, value) => host.setText(node, String(value)),
    isTextNode: (node) => host.isText(node),
    setProperty,
    insertNode: (parent, node, anchor) => host.insertNode(parent, node, anchor),
    removeNode(parent, node) {
      forget(node);
      host.remove(parent, node);
    },
    getParentNode: (node) => defined(host.parent(node)),
    getFirstChild: (node) => defined(host.firstChild(node)),
    getNextSibling: (node) => defined(host.nextSibling(node)),
    retainedCounts: () => ({ nodes: nodes.size }),
    forget,
  };
}
