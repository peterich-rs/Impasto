import { host } from "@impasto/solid-renderer";
import type { PlatformSnapshot } from "./theme.ts";

export type HostExt = typeof host & {
  focused(): number;
  isFocusable(node: number): boolean;
  blur(): void;
  settings(): PlatformSnapshot;
  onSettings(fn: (s: PlatformSnapshot) => void): number;
  offSettings(id: number): void;
  graphemeCount(text: string): number;
  graphemeIndex(text: string, utf16: number): number;
  graphemeUnit(text: string, index: number): number;
  setReverse(node: number, reverse: boolean): void;
  setScaleXY(node: number, scaleX: number, scaleY: number): void;
};

export const coreHost = host as HostExt;

export function nodeId(value: unknown): number {
  if (typeof value === "number") {
    return value;
  }
  if (value !== null && typeof value === "object" && "id" in value) {
    const id = (value as { id: unknown }).id;
    if (typeof id === "number") {
      return id;
    }
  }
  return 0;
}
