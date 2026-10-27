import { log, setListenerErrorPolicy } from "@uifw/solid-renderer";

/** Default listener policy. `freeze` is opt-in and keeps the host on the previous frame. */
setListenerErrorPolicy("report");

export function reportError(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  log("error", message);
}

export function messageOf(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

export function stackOf(error: unknown): string {
  if (error instanceof Error && typeof error.stack === "string") {
    return error.stack;
  }
  return "";
}

/** `window.onerror` and unhandled rejections. Safe to call more than once. */
export function installErrorHooks(): () => void {
  const onError = (event: ErrorEvent) => {
    reportError(event.error ?? event.message);
  };
  const onRejection = (event: PromiseRejectionEvent) => {
    reportError(event.reason);
  };
  if (typeof globalThis.addEventListener === "function") {
    globalThis.addEventListener("error", onError as EventListener);
    globalThis.addEventListener("unhandledrejection", onRejection as EventListener);
  }
  return () => {
    if (typeof globalThis.removeEventListener === "function") {
      globalThis.removeEventListener("error", onError as EventListener);
      globalThis.removeEventListener("unhandledrejection", onRejection as EventListener);
    }
  };
}

export function useErrorHandler(): (error: unknown) => void {
  return (error: unknown) => {
    reportError(error);
  };
}
