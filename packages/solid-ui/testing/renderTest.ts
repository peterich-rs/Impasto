import {installStyleMock} from "../../solid-renderer/testing/style.ts";
import { flush } from "solid-js";
export type Call = { name: string; args: unknown[] };

export type MockHost = {
  calls: Call[];
  host: Record<string, unknown>;
  liveTexts: () => string[];
  clickByLabel: (label: string) => void;
  ancestors: (label: string) => string[];
  flushHostFrame: (targetMs?: number) => void;
  pressBack: () => boolean;
};

const light = {
  locale: "zh-CN",
  textScale: 1,
  brightness: "light" as const,
  reduceMotion: false,
  boldText: false,
  highContrast: false,
};

/** In-memory host used by gallery and solid-ui tests. Assigns `globalThis.core`. */
export function installMockHost(): MockHost {
  const calls: Call[] = [];
  const nodes = new Map<number, { parent: number; children: number[]; kind: string; detached: boolean }>();
  const texts = new Map<number, string>();
  const clicks = new Map<number, () => void>();
  const pendingRemoval = new Set<number>();
  const frames: Array<(targetMs: number, intervalMs: number) => void> = [];
  const motionEnds: Array<{ token: number; fn: () => void }> = [];
  let motionToken = 1;
  let driverNotify:
    | ((
        kind: number,
        index: number,
        generation: number,
        tag: number,
        i0: number,
        i1: number,
        f0: number,
        f1: number,
        f2: number,
        f3: number,
        f4: number,
        f5: number,
        s0: string,
        s1: string,
      ) => void)
    | undefined;
  let driverSerial = 1;
  let motionArmed = false;
  const backs: Array<() => boolean> = [];
  let next = 4294967296;

  function alloc(name: string, args: unknown[]) {
    const id = next++;
    nodes.set(id, { parent: 0, children: [], kind: name, detached: false });
    calls.push({ name, args });
    if (name === "text") {
      texts.set(id, String(args[0] ?? ""));
    }
    return id;
  }

  function unlink(child: number) {
    const node = nodes.get(child);
    if (node === undefined || node.parent === 0) {
      return;
    }
    const parent = nodes.get(node.parent);
    if (parent !== undefined) {
      parent.children = parent.children.filter((id) => id !== child);
    }
    node.parent = 0;
    node.detached = true;
  }

  function dropNode(id: number) {
    const node = nodes.get(id);
    if (node === undefined) {
      return;
    }
    for (const child of [...node.children]) {
      dropNode(child);
    }
    nodes.delete(id);
    texts.delete(id);
  }

  function shown(id: number): boolean {
    let node = id;
    const seen = new Set<number>();
    while (node !== 0) {
      if (seen.has(node)) {
        return false;
      }
      seen.add(node);
      const current = nodes.get(node);
      if (current === undefined || current.detached) {
        return false;
      }
      if (current.parent === 0) {
        return true;
      }
      node = current.parent;
    }
    return false;
  }

  const host = {
    createElement(tag: string, props?: Record<string, unknown>) {
      const id = alloc(tag, tag === "text" ? [props?.value] : []);
      if (props?.style !== undefined) host.setStyle(id, props.style);
      return id;
    },
    createTextNode: (value: string | number) => alloc("text", [value]),
    onDriver: (
      fn: (
        kind: number,
        index: number,
        generation: number,
        tag: number,
        i0: number,
        i1: number,
        f0: number,
        f1: number,
        f2: number,
        f3: number,
        f4: number,
        f5: number,
        s0: string,
        s1: string,
      ) => void,
    ) => {
      driverNotify = fn;
      calls.push({ name: "onDriver", args: ["function"] });
    },
    bindCanvas: (node: number) => {
      calls.push({ name: "bindCanvas", args: [node] });
      return {
        invalidate: () => {},
        toLocal: () => null,
        release: () => {},
      };
    },
    Path2D: class {
      closePath() {}
      moveTo() {}
      lineTo() {}
      quadraticCurveTo() {}
      bezierCurveTo() {}
      rect() {}
      roundRect() {}
      arc() {}
      arcTo() {}
      ellipse() {}
      addPath() {}
    },
    listDriver: () => {
      const index = driverSerial++;
      const generation = 1;
      let visible: { key: string; type: string }[] = [];
      calls.push({ name: "listDriver", args: [index] });
      return {
        bind: () => {},
        configure: () => {},
        setItems: (rows: { key: string; type: string }[]) => {
          const sample = rows.slice(0, 4);
          const previous = visible;
          visible = sample;
          for (const row of previous) {
            if (!sample.some((next) => next.key === row.key)) {
              driverNotify?.(1, index, generation, 1, 1, 0, 0, 0, 0, 0, 0, 0, row.key, "");
            }
          }
          for (let i = 0; i < sample.length; i++) {
            const row = sample[i];
            const op = previous.some((old) => old.key === row.key && old.type === row.type) ? 3 : 2;
            driverNotify?.(1, index, generation, 1, op, i, 0, 0, 0, 0, 0, 0, row.key, row.type);
          }
          for (let i = sample.length - 1; i >= 0; i--) {
            driverNotify?.(1, index, generation, 1, 4, 0, 0, 0, 0, 0, 0, 0, sample[i].key, sample[i + 1]?.key ?? "");
          }
        },
        noteRow: () => {},
        release: () => {},
        id: () => ({ index, generation }),
      };
    },
    scrollController: () => {
      const index = driverSerial++;
      const generation = 1;
      calls.push({ name: "scrollController", args: [index] });
      return {
        bind: () => {},
        configure: () => {},
        dragBy: () => ({ ox: 0, oy: 0, overX: 0, overY: 0, pending: false }),
        finishDrag: () => {},
        dragEnd: () => {},
        wheel: () => {},
        query: () => ({ x: 0, y: 0 }),
        consume: () => false,
        release: () => {},
        id: () => ({ index, generation }),
      };
    },
    append: (parent: number, child: number) => {
      host.insertNode(parent, child);
    },
    insertNode: (parent: number, child: number, before = 0) => {
      const parentNode = nodes.get(parent);
      const childNode = nodes.get(child);
      if (parentNode === undefined || childNode === undefined) {
        throw new Error(`insert missing ${parent} ${child}`);
      }
      unlink(child);
      childNode.parent = parent;
      childNode.detached = false;
      pendingRemoval.delete(child);
      if (before === 0) {
        parentNode.children.push(child);
      } else {
        const index = parentNode.children.indexOf(before);
        if (index < 0) {
          throw new Error("anchor is not a child");
        }
        parentNode.children.splice(index, 0, child);
      }
      calls.push({ name: "insert", args: [parent, child, before] });
    },
    detach: (parent: number, child: number) => {
      const node = nodes.get(child);
      if (node?.parent !== parent) {
        throw new Error("not a child");
      }
      unlink(child);
      calls.push({ name: "detach", args: [parent, child] });
    },
    discard: (id: number) => {
      dropNode(id);
      calls.push({ name: "discard", args: [id] });
    },
    remove: (parent: number, child: number) => {
      const node = nodes.get(child);
      if (node?.parent !== parent) {
        throw new Error("not a child");
      }
      unlink(child);
      pendingRemoval.add(child);
      calls.push({ name: "remove", args: [parent, child] });
    },
    parent: (node: number) => nodes.get(node)?.parent ?? 0,
    firstChild: (node: number) => nodes.get(node)?.children[0] ?? 0,
    nextSibling: (node: number) => {
      const parent = nodes.get(node)?.parent ?? 0;
      if (parent === 0) {
        return 0;
      }
      const children = nodes.get(parent)?.children ?? [];
      const index = children.indexOf(node);
      return children[index + 1] ?? 0;
    },
    isText: (node: number) => nodes.get(node)?.kind === "text",
    setTextRuns: (node: number, runs: { text: string }[]) => {
      texts.set(node, runs.map(run => run.text).join(""));
      calls.push({ name: "setTextRuns", args: [node, runs] });
    },
    setText: (node: number, utf8: string) => {
      if (nodes.get(node)?.kind !== "text") {
        throw new Error(`setText failed: ${node} ${utf8}`);
      }
      texts.set(node, utf8);
      calls.push({ name: "setText", args: [node, utf8] });
    },
    setStyle: (node: number, style: unknown) => {
      calls.push({ name: "setStyle", args: [node, style] });
    },
    patchStyle: (node: number, style: unknown) => {
      calls.push({ name: "patchStyle", args: [node, style] });
    },
    clearStyle: (node: number, keys?: unknown) => {
      calls.push({ name: "clearStyle", args: [node, keys] });
    },
    setClass: (node: number, text: string) => {
      calls.push({ name: "setClass", args: [node, text] });
    },
    setStyleSurface: (node: number, surface: string) => {
      calls.push({ name: "setStyleSurface", args: [node, surface] });
    },
    setComponentStyle: (node: number, style: unknown) => {
      calls.push({ name: "setComponentStyle", args: [node, style] });
    },
    placePanel: (panel: number, anchor: number, align: number) => {
      calls.push({ name: "placePanel", args: [panel, anchor, align] });
    },
    reparentChildren: (from: number, to: number) => {
      calls.push({ name: "reparentChildren", args: [from, to] });
    },
    textCommand: (node: number, name: string, extra?: string) => {
      calls.push({ name: "textCommand", args: [node, name, extra] });
      return { text: extra ?? "", start: 0, end: 0 };
    },
    setTextLimit: (node: number, limit: number) => {
      calls.push({ name: "setTextLimit", args: [node, limit] });
    },
    metrics: () => ({
      width: 360,
      height: 640,
      viewInsets: { top: 0, right: 0, bottom: 0, left: 0 },
    }),
    setImage: (node: number, source: string, width: number, height: number) => {
      calls.push({ name: "setImage", args: [node, source, width, height] });
    },
    setScrollOffset: (node: number, x: number, y: number) => {
      calls.push({ name: "setScrollOffset", args: [node, x, y] });
    },
    overscroll: () => ({ x: 0, y: 0 }),
    setScrollPhysics: () => {},
    setScrollChain: () => {},
    fling: () => {},
    stopScroll: () => {},
    scrollTo: () => {},
    onScrollState: () => {},
    scrollOffset: () => ({ x: 0, y: 0 }),
    rect: () => ({ x: 0, y: 40, w: 100, h: 200 }),
    textEdit: (node: number, op: number, extra: string) => {
      calls.push({ name: "textEdit", args: [node, op, extra] });
      const text = extra ?? "";
      return { text, start: text.length, end: text.length };
    },
    onScroll: () => {},
    setFocusable: (node: number, enabled: boolean) => {
      calls.push({ name: "setFocusable", args: [node, enabled] });
    },
    focus: (node: number) => {
      calls.push({ name: "focus", args: [node] });
    },
    blur: () => {
      calls.push({ name: "blur", args: [] });
    },
    focused: () => 0,
    setImeEnabled: (enabled: boolean) => {
      calls.push({ name: "setImeEnabled", args: [enabled] });
    },
    setImeText: (text: string) => {
      calls.push({ name: "setImeText", args: [text] });
    },
    setImeState: (text: string | null) => {
      calls.push({ name: "setImeState", args: [text] });
    },
    setImeConfig: () => {},
    setSemantics: (...args: unknown[]) => {
      calls.push({ name: "setSemantics", args });
    },
    textScale: () => 1,
    setTextScale: (scale: number) => {
      calls.push({ name: "setTextScale", args: [scale] });
    },
    reduceMotion: () => false,
    measureText: (text: string, fontSize: number, maxWidth: number, color: number[]) => {
      calls.push({ name: "measureText", args: [text, fontSize, maxWidth, color] });
      return { w: text.length, h: fontSize };
    },
    requestFrame: () => {
      calls.push({ name: "requestFrame", args: [] });
    },
    onClick: (node: number, fn: () => void) => {
      clicks.set(node, fn);
      calls.push({ name: "onClick", args: [node, typeof fn] });
    },
    onPress: (node: number, fn: () => void) => {
      calls.push({ name: "onPress", args: [node, typeof fn] });
    },
    onRelease: (node: number, fn: () => void) => {
      calls.push({ name: "onRelease", args: [node, typeof fn] });
    },
    onDragBegin: (node: number, fn: () => void) => {
      calls.push({ name: "onDragBegin", args: [node, typeof fn] });
    },
    onDragBy: (node: number, fn: (dx: number, dy: number) => void) => {
      calls.push({ name: "onDragBy", args: [node, typeof fn] });
    },
    onDragEnd: (node: number, fn: (vx: number, vy: number) => void) => {
      calls.push({ name: "onDragEnd", args: [node, typeof fn] });
    },
    onFocus: (node: number, fn: () => void) => {
      calls.push({ name: "onFocus", args: [node, typeof fn] });
    },
    onBlur: (node: number, fn: () => void) => {
      calls.push({ name: "onBlur", args: [node, typeof fn] });
    },
    onKey: (node: number, fn: (key: string) => void) => {
      calls.push({ name: "onKey", args: [node, typeof fn] });
    },
    onText: (node: number, fn: (text: string) => void) => {
      calls.push({ name: "onText", args: [node, typeof fn] });
    },
    onTextReplaced: (node: number, fn: (text: string) => void) => {
      calls.push({ name: "onTextReplaced", args: [node, typeof fn] });
    },
    onComposing: (node: number, fn: (text: string | null) => void) => {
      calls.push({ name: "onComposing", args: [node, typeof fn] });
    },
    onHover: (node: number, fn: () => void) => {
      calls.push({ name: "onHover", args: [node, typeof fn] });
    },
    onLeave: (node: number, fn: () => void) => {
      calls.push({ name: "onLeave", args: [node, typeof fn] });
    },
    onWheel: (node: number, fn: (dx: number, dy: number) => void) => {
      calls.push({ name: "onWheel", args: [node, typeof fn] });
    },
    onFrame: (fn?: (targetMs: number, intervalMs: number) => void) => {
      frames.length = 0;
      if (fn) frames.push(fn);
      calls.push({ name: "onFrame", args: [typeof fn] });
      return frames.length;
    },
    onLayout: (fn: () => void) => {
      calls.push({ name: "onLayout", args: [typeof fn] });
    },
    setDragClaim: () => {},
    caretAt: () => ({ unit: 0, x: 0, y: 0, w: 1, h: 16 }),
    caretAtUnit: () => ({ x: 0, y: 0, w: 1, h: 16 }),
    selectionRects: () => [],
    wordRange: () => ({ start: 0, end: 0 }),
    focusStep: () => {},
    pushFocusScope: () => {},
    popFocusScope: () => {},
    pointerPosition: () => ({ x: 0, y: 0 }),
    onBack: (fn: () => boolean) => {
      backs.push(fn);
      return backs.length;
    },
    offBack: (id: number) => {
      if (id > 0 && id <= backs.length) {
        backs[id - 1] = () => false;
      }
    },
    onZoom: (node: number, fn: (scale: number) => void) => {
      calls.push({ name: "onZoom", args: [node, typeof fn] });
    },
    onShortcut: (spec: string, cb: () => void) => {
      calls.push({ name: "onShortcut", args: [spec, typeof cb] });
    },
    scrollIntoView: (id: number, edge: number) => {
      calls.push({ name: "scrollIntoView", args: [id, edge] });
    },
    animate: (id: number, prop: string, from: number, to: number, stiffness: number, damping: number) => {
      calls.push({ name: "animate", args: [id, prop, from, to, stiffness, damping] });
      motionArmed = true;
    },
    onMotionEnd: (id: number, prop: string, fn: () => void) => {
      const token = motionToken;
      motionToken += 1;
      motionEnds.push({ token, fn });
      calls.push({ name: "onMotionEnd", args: [id, prop, typeof fn] });
      return token;
    },
    offMotionEnd: (token: number) => {
      const index = motionEnds.findIndex((item) => item.token === token);
      if (index >= 0) {
        motionEnds.splice(index, 1);
      }
    },
    flingTarget: () => null,
    setListenerErrorPolicy: (policy: string) => {
      calls.push({ name: "setListenerErrorPolicy", args: [policy] });
    },
    layerTreeStats: () => ({ epoch: 1, layers: 0, pictures: 0 }),
    setRecognizers: (node: number, spec: unknown) => {
      calls.push({ name: "setRecognizers", args: [node, spec] });
    },
    log: (level: string, message: string) => {
      calls.push({ name: "log", args: [level, message] });
    },
    debugLayoutIssues: () => "{\"issues\":[]}",
    debugSnapshot: () => "null",
    debugProbe: () => "{\"chain\":[]}",
    debugLastPointer: () => "null",
    debugSetScan: () => undefined,
    debugSetPointerTrace: () => undefined,
    debugSetOwned: () => undefined,
    debugMemory: () => "{\"mallocBytes\":0,\"objects\":0}",
    rect: () => ({ x: 0, y: 0, w: 0, h: 0 }),
    getInitialURL: () => Promise.resolve(null),
    onURL: () => {},
    settings: () => light,
    setStyleEnvironment: (environment: unknown) => {
      calls.push({ name: "setStyleEnvironment", args: [environment] });
    },
    onSettings: () => 1,
    offSettings: () => {},
    metrics: () => ({
      width: 360,
      height: 640,
      viewInsets: { top: 0, right: 0, bottom: 0, left: 0 },
    }),
  };

  installStyleMock(host,calls);
  (globalThis as unknown as { core: typeof host }).core = host;

  function ancestors(label: string): string[] {
    const lines: string[] = [];
    for (const [id, value] of texts) {
      if (value !== label || (nodes.get(id)?.parent ?? 0) === 0) {
        continue;
      }
      const chain: string[] = [];
      let node = id;
      while (node !== 0) {
        chain.push(`${nodes.get(node)?.kind ?? "?"}#${node}`);
        node = nodes.get(node)?.parent ?? 0;
      }
      lines.push(chain.join(" < "));
    }
    return lines;
  }

  function liveTexts(): string[] {
    flush();
    const values: string[] = [];
    for (const [id, value] of texts) {
      if ((nodes.get(id)?.parent ?? 0) !== 0 && shown(id) && value.length > 0) {
        values.push(value);
      }
    }
    return values;
  }

  function clickByLabel(label: string): void {
    for (const [id, value] of texts) {
      if (value !== label || !shown(id)) {
        continue;
      }
      let node = id;
      while (node !== 0) {
        const fn = clicks.get(node);
        if (fn) {
          fn();
          return;
        }
        node = nodes.get(node)?.parent ?? 0;
      }
    }
    throw new Error(`no click for ${label}`);
  }

  function flushHostFrame(targetMs = 0): void {
    flush();
    if (motionArmed) {
      motionArmed = false;
      const batch = motionEnds.splice(0);
      for (const item of batch) {
        item.fn();
      }
    }
    for (const fn of [...frames]) {
      fn(targetMs, 16);
      flush();
    }
    for (const id of pendingRemoval) dropNode(id);
    pendingRemoval.clear();
  }

  function pressBack(): boolean {
    const ordered = backs.slice().reverse();
    for (const fn of ordered) {
      if (fn()) {
        return true;
      }
    }
    return false;
  }

  return { calls, host, liveTexts, clickByLabel, flushHostFrame, pressBack, ancestors };
}
