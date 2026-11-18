import * as native from "./native.js";

export const AppState = {
  get currentState() {
    return native.getCurrentState();
  },
  addEventListener: native.addEventListener,
  removeEventListener: native.removeEventListener,
};
