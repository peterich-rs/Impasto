/** Version 1 scene bytes. The magic is UIFW. See docs/native-phase.md. */

export const KIND_LAYER = 1;
export const KIND_RECT = 2;
export const KIND_TEXT = 3;
export const KIND_IMAGE = 4;

export interface SceneNode {
  id: number;
  parentId: number;
  kind: number;
  x: number;
  y: number;
  w: number;
  h: number;
  r: number;
  g: number;
  b: number;
  a: number;
  text: string;
  children: SceneNode[];
}

export interface Scene {
  nodes: readonly SceneNode[];
  layer(parent: number): number;
  rect(parent: number, x: number, y: number, w: number, h: number, color: Rgba): number;
  text(parent: number, x: number, y: number, text: string, color: Rgba): number;
  image(parent: number, x: number, y: number, w: number, h: number, source: string): number;
  node(id: number): SceneNode;
  encode(): Uint8Array;
}

export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

const textEncoder = new TextEncoder();

class Writer {
  private data = new Uint8Array(256);
  private size = 0;

  u8(value: number) {
    if (this.size === this.data.length) {
      const grown = new Uint8Array(this.data.length * 2);
      grown.set(this.data);
      this.data = grown;
    }
    this.data[this.size++] = value & 0xff;
  }

  u32(value: number) {
    this.u8(value);
    this.u8(value >>> 8);
    this.u8(value >>> 16);
    this.u8(value >>> 24);
  }

  f32(value: number) {
    const bits = new Uint8Array(4);
    new DataView(bits.buffer).setFloat32(0, value, true);
    for (const byte of bits) {
      this.u8(byte);
    }
  }

  bytes(value: Uint8Array) {
    for (const byte of value) {
      this.u8(byte);
    }
  }

  toUint8Array(): Uint8Array {
    return this.data.slice(0, this.size);
  }
}

function kindName(kind: number): string {
  switch (kind) {
    case KIND_LAYER:
      return "layer";
    case KIND_RECT:
      return "rect";
    case KIND_TEXT:
      return "text";
    case KIND_IMAGE:
      return "image";
    default:
      return "unknown";
  }
}

export function createScene(): Scene {
  const nodes: SceneNode[] = [];

  function append(
    kind: number,
    parent: number,
    x: number,
    y: number,
    w: number,
    h: number,
    color: Rgba,
    text: string,
  ): SceneNode {
    const node: SceneNode = {
      id: nodes.length + 1,
      parentId: parent,
      kind,
      x,
      y,
      w,
      h,
      r: color.r,
      g: color.g,
      b: color.b,
      a: color.a,
      text,
      children: [],
    };
    nodes.push(node);
    return node;
  }

  return {
    nodes,
    layer(parent) {
      return append(KIND_LAYER, parent, 0, 0, 0, 0, { r: 0, g: 0, b: 0, a: 0 }, "").id;
    },
    rect(parent, x, y, w, h, color) {
      return append(KIND_RECT, parent, x, y, w, h, color, "").id;
    },
    text(parent, x, y, text, color) {
      return append(KIND_TEXT, parent, x, y, 0, 0, color, text).id;
    },
    image(parent, x, y, w, h, source) {
      return append(KIND_IMAGE, parent, x, y, w, h, { r: 0, g: 0, b: 0, a: 0 }, source).id;
    },
    node(id) {
      const node = nodes[id - 1];
      if (node === undefined || node.id !== id) {
        throw new Error(`unknown node ${id}`);
      }
      return node;
    },
    encode() {
      const out = new Writer();
      out.u8("U".charCodeAt(0));
      out.u8("I".charCodeAt(0));
      out.u8("F".charCodeAt(0));
      out.u8("W".charCodeAt(0));
      out.u32(1);
      out.u32(nodes.length);
      for (const node of nodes) {
        const utf8 = textEncoder.encode(node.text);
        out.u32(node.id);
        out.u32(node.parentId);
        out.u8(node.kind);
        out.u8(0);
        out.u8(0);
        out.u8(0);
        out.f32(node.x);
        out.f32(node.y);
        out.f32(node.w);
        out.f32(node.h);
        out.u8(node.r);
        out.u8(node.g);
        out.u8(node.b);
        out.u8(node.a);
        out.u32(utf8.length);
        out.bytes(utf8);
      }
      return out.toUint8Array();
    },
  };
}

export function counterScene(): Scene {
  const scene = createScene();
  const layer = scene.layer(0);
  scene.rect(layer, 0, 0, 320, 180, { r: 17, g: 17, b: 17, a: 255 });
  scene.text(layer, 24, 36, "Count", { r: 255, g: 255, b: 255, a: 255 });
  scene.text(layer, 24, 72, "0", { r: 124, g: 255, b: 107, a: 255 });
  scene.image(layer, 240, 24, 56, 56, "asset:logo");
  return scene;
}

export function formatScene(scene: Scene): string {
  const lines = [`UIFW version=1 nodes=${scene.nodes.length}`];
  for (const node of scene.nodes) {
    lines.push(
      `${node.id} ${kindName(node.kind)} parent=${node.parentId} box=${node.x},${node.y},${node.w},${node.h} rgba=${node.r},${node.g},${node.b},${node.a} "${node.text}"`,
    );
  }
  return lines.join("\n");
}
