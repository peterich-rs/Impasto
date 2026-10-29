import * as host from "@uifw/app-state/host";

export const AppState = {
  get currentState() {
    return host.getCurrentState();
  },
  addEventListener: host.addEventListener,
  removeEventListener: host.removeEventListener,
};
