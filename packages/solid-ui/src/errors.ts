import { log, setListenerErrorPolicy } from "@impasto/solid-renderer";

/** Default listener policy. `freeze` is opt-in and keeps the host on the previous frame. */
setListenerErrorPolicy("report");

export function reportError(error: unknown): void {
  log("error", formatError(error));
}

/** Message plus stack, so a host log line can be copied without the error page. */
export function formatError(error: unknown): string {
  if (!(error instanceof Error)) {
    return String(error);
  }
  const stack = typeof error.stack === "string" ? error.stack : "";
  if (stack.length === 0) {
    return error.message;
  }
  return stack.includes(error.message) ? stack : `${error.message}\n${stack}`;
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
  const global = globalThis as {
    addEventListener?: typeof globalThis.addEventListener;
    removeEventListener?: typeof globalThis.removeEventListener;
    __reportUnhandled?: (text: string) => void;
  };
  if (typeof global.addEventListener === "function") {
    global.addEventListener("error", onError as EventListener);
    global.addEventListener("unhandledrejection", onRejection as EventListener);
  } else {
    // QuickJS: the glue-layer rejection checker calls this hook with the
    // formatted reason and stack instead of a DOM event.
    global.__reportUnhandled = (text: string) => {
      log("error", text);
    };
  }
  return () => {
    if (typeof global.removeEventListener === "function") {
      global.removeEventListener("error", onError as EventListener);
      global.removeEventListener("unhandledrejection", onRejection as EventListener);
    } else {
      delete global.__reportUnhandled;
    }
  };
}

export function useErrorHandler(): (error: unknown) => void {
  return (error: unknown) => {
    reportError(error);
  };
}
