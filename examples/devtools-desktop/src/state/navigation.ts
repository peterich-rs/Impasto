import { createSignal } from "solid-js";

export type NavigationIntent =
  | { kind: "overview" }
  | { kind: "frame"; frameId?: string; tab?: "breakdown" | "repaints" | "js" }
  | { kind: "view"; nodeId?: string; mode?: "tree" | "leaderboard" | "layout" | "layers"; highlight?: boolean; fromFrameId?: string }
  | { kind: "js"; subTab?: "flamegraph" | "memory" | "resources" | "console"; frameId?: string; search?: string }
  | { kind: "network"; token?: string; tab?: "timing" | "headers" | "body" | "ws"; status?: string }
  | { kind: "resources"; subTab?: "trends" | "bundle" | "archive" }
  | { kind: "console"; frameId?: string; errorSeq?: string };

export type Page = Exclude<NavigationIntent["kind"], "console">;
export const pages: readonly { id: Page; label: string }[] = [
  { id: "overview", label: "Overview" }, { id: "frame", label: "Frames" },
  { id: "view", label: "Views" }, { id: "js", label: "JS Runtime" },
  { id: "network", label: "Network" }, { id: "resources", label: "Resources" },
];

export function navigation(apply: (intent: NavigationIntent) => void) {
  let entries: NavigationIntent[] = [{ kind: "overview" }];
  let index = 0;
  const [position, setPosition] = createSignal({ index, total: 1 });
  return {
    position,
    go(intent: NavigationIntent) {
      entries = [...entries.slice(0, index + 1), intent].slice(-64);
      index = entries.length - 1;
      setPosition({ index, total: entries.length });
      apply(intent);
    },
    step(delta: number) {
      const next = Math.max(0, Math.min(entries.length - 1, index + delta));
      if (next === index) return;
      index = next;
      setPosition({ index, total: entries.length });
      apply(entries[index]);
    },
  };
}
