import { host } from "@uifw/solid-renderer";

export function moveChildren(from: number, to: number, skip: ReadonlySet<number> = new Set()): void {
  const kids: number[] = [];
  let child = host.firstChild(from);
  while (child !== 0) {
    const next = host.nextSibling(child);
    if (!skip.has(child)) {
      kids.push(child);
    }
    child = next;
  }
  for (const id of kids) {
    host.insert(to, id, 0);
  }
}
