declare module "impasto:test" {
  type Node = number;
  type Rect = { x: number; y: number; w: number; h: number };
  type Role = "generic" | "text" | "button" | "image" | "scroll" | "switch" | "textbox" | "searchbox" | "textarea";
  type Counters = {
    createdNodes: number;
    ffiBatches: number;
    ffiCalls: number;
    styleResolves: number;
    repaintBoundariesDirtied: number;
    updates: number;
    sets: number;
    layoutPasses: number;
    textShapings: number;
    pointers: number;
    edits: number;
  };
  export function test(name: string, run: () => unknown): void;
  export function describe(name: string, register: () => void): void;
  type ExpectedError = RegExp | (new (...args: any[]) => Error) | ((error: unknown) => boolean);
  export const assert: {
    fail(message?: string): never;
    ok(value: unknown, message?: string): asserts value;
    equal(actual: unknown, expected: unknown, message?: string): void;
    notEqual(actual: unknown, expected: unknown, message?: string): void;
    deepEqual(actual: unknown, expected: unknown, message?: string): void;
    match(actual: string, pattern: RegExp, message?: string): void;
    doesNotMatch(actual: string, pattern: RegExp, message?: string): void;
    throws(work: () => unknown, expected?: ExpectedError): void;
    doesNotThrow(work: () => unknown): void;
    rejects(work: PromiseLike<unknown> | (() => PromiseLike<unknown>), expected?: ExpectedError): Promise<void>;
  };
  export function expect<T>(value: T): {
    toBe(value: T): void;
    toEqual(value: unknown): void;
    toBeTruthy(): void;
    toBeFalsy(): void;
    toBeLessThan(value: number): void;
    toBeGreaterThan(value: number): void;
    toBeVisible(): void;
  };
  export function advanceFrames(count?: number, step?: number): Promise<void>;
  export const clock: {
    install(): Promise<void>;
    advance(milliseconds: number): Promise<void>;
  };
  export const screen: {
    mount(render: () => unknown): Promise<void>;
    byRole(role: Role, options?: { name?: string }): Promise<Node>;
    byLabel(label: string): Promise<Node>;
    byText(text: string): Promise<Node>;
    nodes(role?: Role | "", label?: string | null): Promise<{ id: Node; label: string }[]>;
    rect(node: Node): Rect;
    viewport(width: number, height: number, scale?: number): Promise<void>;
    snapshot(name: string): Promise<void>;
    advanceFrames: typeof advanceFrames;
  };
  export const fire: {
    click(node: Node): Promise<void>;
    focus(node: Node): Promise<void>;
    wheel(node: Node, dx: number, dy: number): Promise<void>;
    drag(node: Node, dx: number, dy: number): Promise<void>;
    back(phase?: "start" | "progress" | "cancel" | "commit", value?: number): Promise<boolean>;
    replaceText(node: Node, text: string): Promise<void>;
    imeCompose(node: Node, text: string, start: number, end: number): Promise<void>;
  };
  export function trace(work: () => unknown): Promise<Counters>;
}
declare module "impasto:test/fixtures" {
  export function readText(relativePath: string): string;
}
