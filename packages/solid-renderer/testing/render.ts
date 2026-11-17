// The renderer Babel's universal output calls. Same contract as
// `@solidjs/universal`'s `createRenderer`: `insert` reconciles children, `spread`
// writes props, `effect` and `memo` are render effects. The graph is the native
// one. This file does not import `@solidjs/universal`.

// QuickJS has the native module. Node tests do not, and they compare against the
// installed solid-js, so the fallback is that package. The pack step aliases
// `solid-js` to the shim, so a bundle never reaches the fallback.
import { createMemo, createRenderEffect, createRoot, untrack, runWithOwner } from "solid-js";
import { createComponent as makeComponent, flatten, merge } from "../src/../testing/signals.ts";
import type { Node, SceneOptions } from "../src/tree.ts";

const INNER_OWNED = {};

type EffectOptions = { name?: string; scope?: boolean; schedule?: boolean; onUpdate?: (value: unknown) => void };

function effect<T>(
  compute: (prev?: T) => T,
  apply: (value: T, prev?: T) => void,
  options?: EffectOptions,
): void {
  // `schedule` defers the first run to the next flush. A slot renders inside a
  // tracked effect, and a render effect that runs immediately there is disposed
  // before it can subscribe. universal passes the same option through.
  const run = options?.schedule
    ? { schedule: true as const, transparent: !options.scope }
    : { sync: true as const, transparent: !options?.scope };
  (createRenderEffect as (c: typeof compute, e: typeof apply, o: object) => void)(compute, apply, run);
}

function memo<T>(fn: () => T): () => T {
  return createMemo(fn);
}

export function createSceneRenderer(options: SceneOptions) {
  const {
    createElement,
    createTextNode,
    createSentinel = () => createTextNode(""),
    isTextNode,
    replaceText,
    insertNode,
    removeNode,
    cleanupNodes,
    setProperty,
    getParentNode,
    getFirstChild,
    getNextSibling,
  } = options;

  function insert(
    parent: Node,
    accessor: unknown,
    marker?: Node | null,
    initial?: unknown,
    effectOptions?: EffectOptions,
  ): void {
    const onUpdate = effectOptions?.onUpdate;
    const multi = marker !== undefined;
    if (multi && !initial) initial = [];
    if (typeof accessor !== "function") {
      accessor = normalize(accessor, multi, true);
      if (typeof accessor !== "function") {
        insertExpression(parent, accessor, initial, marker);
        onUpdate?.(accessor);
        return;
      }
    }
    if (multi && Array.isArray(initial) && initial.length === 0) {
      const sentinel = createSentinel();
      insertNode(parent, sentinel, marker ?? undefined);
      initial = [sentinel];
    }
    let current = initial;
    effect(
      (prev) => {
        const value = normalize((accessor as () => unknown)(), multi, true);
        if (typeof value !== "function") return value as never;
        effect(
          () => normalize(value, multi) as never,
          (inner) => {
            insertExpression(parent, inner, current, marker);
            current = inner;
            onUpdate?.(current);
          },
          prev !== undefined && !effectOptions?.schedule ? { schedule: true } : effectOptions,
        );
        return INNER_OWNED as never;
      },
      (value) => {
        if (value === INNER_OWNED) return;
        insertExpression(parent, value, current, marker);
        current = value;
        onUpdate?.(current);
      },
    );
  }

  function insertExpression(parent: Node, value: unknown, current: unknown, marker?: Node | null): void {
    if (value === current) return;
    const kind = typeof value;
    const multi = marker !== undefined;
    if (kind === "string" || (kind === "number" && Number(value) < 4294967296)) {
      const previous = typeof current;
      if (previous === "string" || previous === "number") {
        const child = getFirstChild(parent);
        if (child) replaceText(child, value as string | number);
      } else {
        cleanChildren(parent, current, marker, createTextNode(value as string | number));
      }
    } else if (value == null) {
      cleanChildren(parent, current, marker);
    } else if (Array.isArray(value)) {
      if (value.length === 0) {
        cleanChildren(parent, current, marker);
      } else if (Array.isArray(current)) {
        if (current.length === 0) appendNodes(parent, value, marker ?? undefined);
        else reconcileArrays(parent, current as Node[], value as Node[]);
      } else if (current == null) {
        appendNodes(parent, value);
      } else {
        const first = getFirstChild(parent);
        reconcileArrays(parent, ((multi && current) || (first ? [first] : [])) as Node[], value as Node[]);
      }
    } else if (Array.isArray(current)) {
      cleanChildren(parent, current, multi ? marker : null, value as Node);
    } else if (current == null || !getFirstChild(parent)) {
      insertNode(parent, value as Node);
    } else {
      const first = getFirstChild(parent);
      if (first) replaceNode(parent, value as Node, first);
    }
  }

  function normalize(value: unknown, multi: boolean, doNotUnwrap?: boolean): unknown {
    // A component takes props. A zero-arg function here is a JSX element the
    // compiler wrapped, or a renderer memo. Both produce a node when called.
    const mount = (item: unknown): unknown =>
      typeof item === "function" ? (item as (props?: Record<string, unknown>) => unknown)({}) : item;
    value = mount(value);
    value = flatten(value, { skipNonRendered: true, doNotUnwrap });
    if (Array.isArray(value)) value = value.map(mount);
    if (doNotUnwrap && typeof value === "function") return value;
    if (multi && !Array.isArray(value)) value = [value != null ? value : ""];
    if (Array.isArray(value)) {
      for (let index = 0; index < value.length; index += 1) {
        const item = value[index];
        if (typeof item === "string" || (typeof item === "number" && item < 4294967296)) {
          value[index] = createTextNode(item);
        }
      }
    }
    return value;
  }

  function reconcileArrays(parent: Node, previous: Node[], next: Node[]): void {
    let previousEnd = previous.length;
    let nextEnd = next.length;
    const nextLength = next.length;
    let previousStart = 0;
    let nextStart = 0;
    const after = getNextSibling(previous[previousEnd - 1]);
    let map: Map<Node, number> | null = null;
    const live = (node: Node | undefined) => node != null && getParentNode(node) === parent;

    while (previousStart < previousEnd || nextStart < nextEnd) {
      if (previous[previousStart] === next[nextStart] && live(previous[previousStart])) {
        previousStart += 1;
        nextStart += 1;
        continue;
      }
      while (previous[previousEnd - 1] === next[nextEnd - 1] && live(previous[previousEnd - 1])) {
        previousEnd -= 1;
        nextEnd -= 1;
      }
      if (previousEnd === previousStart) {
        const anchor =
          nextEnd < nextLength
            ? nextStart
              ? getNextSibling(next[nextStart - 1])
              : next[nextEnd - nextStart]
            : after;
        while (nextStart < nextEnd) insertNode(parent, next[nextStart++], anchor);
      } else if (nextEnd === nextStart) {
        while (previousStart < previousEnd) {
          if (!map || !map.has(previous[previousStart])) removeNode(parent, previous[previousStart]);
          previousStart += 1;
        }
      } else if (
        previous[previousStart] === next[nextEnd - 1] &&
        next[nextStart] === previous[previousEnd - 1]
      ) {
        const anchor = previous[previousStart];
        do {
          insertNode(parent, previous[--previousEnd], anchor);
          nextStart += 1;
          if (previousStart >= previousEnd - 1 || nextStart >= nextEnd) break;
        } while (
          previous[previousStart] === next[nextEnd - 1] &&
          next[nextStart] === previous[previousEnd - 1]
        );
      } else {
        if (!map) {
          map = new Map();
          let index = nextStart;
          while (index < nextEnd) map.set(next[index], index++);
        }
        const index = map.get(previous[previousStart]);
        if (index != null) {
          if (nextStart < index && index < nextEnd) {
            let cursor = previousStart;
            let sequence = 1;
            while (++cursor < previousEnd && cursor < nextEnd) {
              const found = map.get(previous[cursor]);
              if (found == null || found !== index + sequence) break;
              sequence += 1;
            }
            if (sequence > index - nextStart) {
              const node = previous[previousStart];
              while (nextStart < index) insertNode(parent, next[nextStart++], node);
            } else {
              replaceNode(parent, next[nextStart++], previous[previousStart++]);
            }
          } else previousStart += 1;
        } else removeNode(parent, previous[previousStart++]);
      }
    }
  }

  function cleanChildren(parent: Node, current: unknown, marker?: Node | null, replacement?: Node): void {
    if (marker === undefined) {
      let removed = getFirstChild(parent);
      while (removed) {
        removeNode(parent, removed);
        removed = getFirstChild(parent);
      }
      if (replacement) insertNode(parent, replacement);
      return;
    }
    if (Array.isArray(current) && current.length) {
      let inserted = false;
      for (let index = current.length - 1; index >= 0; index -= 1) {
        const node = current[index] as Node;
        if (replacement !== node) {
          const isParent = getParentNode(node) === parent;
          if (replacement && !inserted && index === 0) {
            if (isParent) replaceNode(parent, replacement, node);
            else insertNode(parent, replacement, marker ?? undefined);
          } else if (isParent) removeNode(parent, node);
        } else inserted = true;
      }
    } else if (replacement) insertNode(parent, replacement, marker ?? undefined);
  }

  function appendNodes(parent: Node, nodes: Node[], marker?: Node): void {
    for (const node of nodes) insertNode(parent, node, marker);
  }

  function replaceNode(parent: Node, next: Node, previous: Node): void {
    insertNode(parent, next, previous);
    removeNode(parent, previous);
  }

  function collectNodes(value: unknown, nodes: Node[]): Node[] {
    if (Array.isArray(value)) {
      for (const item of value) collectNodes(item, nodes);
    } else if (value != null && typeof value !== "string" && (typeof value !== "number" || value >= 4294967296)) {
      nodes.push(value as Node);
    }
    return nodes;
  }

  function collectMounted(parent: Node, value: unknown): Node[] {
    const nodes = collectNodes(value, []);
    if (!nodes.length && (typeof value === "string" || typeof value === "number")) {
      const node = getFirstChild(parent);
      if (node) nodes.push(node);
    }
    return nodes;
  }

  function defaultCleanup(parent: Node, nodes: Node[]): void {
    for (const node of nodes) {
      if (getParentNode(node) === parent) removeNode(parent, node);
    }
  }

  function spread(node: Node, props: unknown, skipChildren?: boolean): Record<string, unknown> {
    const previous: Record<string, unknown> = {};
    const apply = (next: Record<string, unknown>) => {
      for (const prop in previous) {
        if (prop in next) continue;
        if (prop !== "ref") setProperty(node, prop, undefined, previous[prop]);
        delete previous[prop];
      }
      for (const prop in next) {
        const value = next[prop];
        if (value === previous[prop]) continue;
        if (prop === "ref") {
          if (typeof value === "function" || Array.isArray(value)) ref(() => value, node);
        } else setProperty(node, prop, value, previous[prop]);
        previous[prop] = value;
      }
    };
    const sources = Array.isArray(props) ? props : [props];
    if (!skipChildren) {
      insert(node, () => {
        for (let index = sources.length - 1; index >= 0; index -= 1) {
          const source = resolveSource(sources[index]);
          if (source != null && typeof source === "object" && "children" in source) {
            return (source as { children?: unknown }).children;
          }
        }
        return undefined;
      });
    }
    effect(
      () => collectSources(sources) as never,
      (next) => apply(next as Record<string, unknown>),
    );
    return previous;
  }

  function resolveSource(source: unknown): unknown {
    return typeof source === "function" ? (source as () => unknown)() : source;
  }

  function collectSources(sources: unknown[]): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    const resolved = sources.map(resolveSource).filter((source) => source != null);
    for (let index = 0; index < resolved.length; index += 1) {
      const source = resolved[index] as Record<string, unknown>;
      for (const key of Object.keys(source)) {
        if (key === "children") continue;
        let shadowed = false;
        for (let later = index + 1; later < resolved.length; later += 1) {
          if (key in (resolved[later] as object)) {
            shadowed = true;
            break;
          }
        }
        if (!shadowed) out[key] = source[key];
      }
    }
    return out;
  }

  function applyRef(refValue: unknown, element: Node): void {
    const list = Array.isArray(refValue) ? refValue.flat(Infinity) : [refValue];
    for (const item of list) {
      if (typeof item === "function") (item as (element: Node) => void)(element);
    }
  }

  function ref(fn: () => unknown, element: Node): void {
    const resolved = untrack(fn);
    runWithOwner(null, () => applyRef(resolved, element));
  }

  return {
    render(code: () => Node, element: Node): () => void {
      let dispose: () => void = () => {};
      let disposed = false;
      let mounted: Node[] = [];
      const cleanup = cleanupNodes ?? defaultCleanup;
      try {
        createRoot((done) => {
          dispose = done;
          const tree = code();
          insert(element, () => tree, undefined, undefined, {
            onUpdate(value) {
              mounted = collectMounted(element, value);
            },
          });
        });
      } catch (error) {
        dispose();
        cleanup(element, mounted);
        throw error;
      }
      return () => {
        if (disposed) return;
        disposed = true;
        dispose();
        cleanup(element, mounted);
        mounted = [];
      };
    },
    insert,
    spread,
    createElement,
    createTextNode,
    insertNode,
    setProp<T>(node: Node, name: string, value: T, prev?: T): T {
      setProperty(node, name, value, prev);
      return value;
    },
    mergeProps: merge,
    createComponent: makeComponent,
    effect,
    memo,
    applyRef,
    ref,
    isTextNode,
  };
}
