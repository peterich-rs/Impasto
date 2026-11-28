export type OpenUrlMode = "external" | "inAppBrowser";
export type LinkingErrorCode = "invalid-url" | "unsupported-scheme" | "cannot-open" | "host-unavailable" | "platform";
export interface OpenUrlOptions {
  readonly mode?: OpenUrlMode;
  readonly toolbarColor?: string;
  readonly dismissButtonStyle?: "done" | "close" | "cancel";
}
export type UrlListener = (payload: { url: string }) => void;
