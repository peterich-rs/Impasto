import * as scene from "./app-state.ts";

export const AppState = {
  get currentState() {
    return scene.getCurrentState();
  },
  addEventListener: scene.addEventListener,
  removeEventListener: scene.removeEventListener,
};
