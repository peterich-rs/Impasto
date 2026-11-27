import type { DirectoryScope } from "@impasto/file-system";
export type ShareStatus = "success" | "dismissed" | "unavailable";
export type ShareErrorCode = "invalid-input" | "file-not-found" | "outside-scope" | "busy" | "host-unavailable" | "platform";
export type ShareFileInput =
  | { readonly path: string; readonly scope?: DirectoryScope; readonly mimeType?: string; readonly name?: string }
  | { readonly name: string; readonly bytes: Uint8Array; readonly mimeType?: string };
export interface ShareAnchor { readonly x: number; readonly y: number; readonly width: number; readonly height: number }
export interface ShareOptions {
  readonly text?: string;
  readonly url?: string;
  readonly title?: string;
  readonly subject?: string;
  readonly files?: readonly ShareFileInput[];
  readonly anchor?: ShareAnchor;
}
export interface ShareResult { readonly status: ShareStatus; readonly activity?: string }
