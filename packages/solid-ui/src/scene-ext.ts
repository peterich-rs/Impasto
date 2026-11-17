import { scene } from "@impasto/solid-renderer";
import type { PlatformSnapshot } from "./theme.ts";
export type SceneExtension = typeof scene & {
  focused(): number;
  isFocusable(node: number): boolean;
  blur(): void;
  settings(): PlatformSnapshot;
  onSettings(fn: (s: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
  graphemeCount(text: string): number;
  graphemeIndex(text: string, utf16: number): number;
  graphemeUnit(text: string, index: number): number;
};
export const sceneApi = scene as SceneExtension;
export function nodeId(value: unknown): number {
  if (typeof value === "number") {
    return value;
  }
  return 0;
}
