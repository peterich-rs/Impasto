import * as native from "./native.js";

export const AppState = {
  get currentState() {
    return native.getCurrentState();
  },
  addEventListener: native.addEventListener,
  removeEventListener: native.removeEventListener,
};

/** Small, task-scoped UI state. Values are available during application startup. */
export const Restoration = {
  get: native.getRestorationState,
  set: native.setRestorationState,
};

/** Explicit shared application values across the windows of a live host session. */
export const WindowState = {
  get: native.getWindowState,
  set: native.setWindowState,
  addEventListener: (listener: (key: string) => void) => native.addEventListener("windowState", listener),
  removeEventListener: (listener: (key: string) => void) => native.removeEventListener("windowState", listener),
};
