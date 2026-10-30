import { host } from "./host.ts";

type Entry = { priority: number; seq: number; fn: () => boolean };

let entries: Entry[] = [];
let seq = 0;
let hostId = 0;
let installed = false;

/**
 * Register `fn`. Higher `priority` runs first. The same priority runs
 * latest-first. Returns a release function. One `host.onBack` for the process.
 */
export function onBack(priority: number, fn: () => boolean): () => void {
  if (typeof host.onBack !== "function") {
    return () => {};
  }
  if (!installed) {
    hostId = host.onBack(() => {
      const ordered = entries.slice().sort((left, right) => {
        if (left.priority !== right.priority) {
          return right.priority - left.priority;
        }
        return right.seq - left.seq;
      });
      for (const entry of ordered) {
        if (entry.fn()) {
          return true;
        }
      }
      return false;
    });
    installed = true;
  }
  const mine = ++seq;
  entries.push({ priority, seq: mine, fn });
  return () => {
    entries = entries.filter((entry) => entry.seq !== mine);
    if (entries.length === 0 && installed && typeof host.offBack === "function") {
      host.offBack(hostId);
      installed = false;
      hostId = 0;
    }
  };
}
