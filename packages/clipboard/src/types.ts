export type ClipboardFormat = "text" | "html" | "uri" | "image";
export type ClipboardErrorCode = "empty-content" | "invalid-image" | "host-unavailable" | "platform";

export interface ClipboardContent {
  readonly text?: string;
  readonly html?: string;
  readonly uri?: string;
  readonly imagePng?: Uint8Array;
}
