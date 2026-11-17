import type { StyleInput } from "../src/style.ts";

/** Records source ownership for Node tests; native fixtures verify resolution and layout. */
export function installStyleMock(
  host: object,
  calls: { name: string; args: unknown[] }[],
): void {
  let next = 1;
  const sources = new Map<
    number,
    { node: number; origin: string; style: StyleInput | string }
  >();
  Object.assign(host, {
    createStyleSource(node: number, origin: string) {
      const source = next++;
      sources.set(source, { node, origin, style: null });
      calls.push({ name: "createStyleSource", args: [node, origin, source] });
      return source;
    },
    setStyleSource(source: number, style: StyleInput | string) {
      const entry = sources.get(source);
      if (!entry) throw new Error("stale style source");
      entry.style = style;
      calls.push({
        name: "setStyleSource",
        args: [entry.node, entry.origin, style],
      });
    },
    removeStyleSource(source: number) {
      sources.delete(source);
      calls.push({ name: "removeStyleSource", args: [source] });
    },
  });
}
