import type { Linking as LinkingApi } from "./native.js";

export const state: {
  initialUrl: string | null;
  urlListener: ((event: { url: string }) => void) | null;
} = {
  initialUrl: null,
  urlListener: null,
};

export function getInitialURL(): string | null {
  return state.initialUrl;
}

export function openURL(_url: string): Promise<void> {
  return Promise.resolve();
}

export const addEventListener: LinkingApi["addEventListener"] = (type, listener) => {
  if (type === "url") {
    state.urlListener = listener;
  }
};

export const removeEventListener: LinkingApi["removeEventListener"] = (type, listener) => {
  if (type === "url" && state.urlListener === listener) {
    state.urlListener = null;
  }
};

export const Linking: LinkingApi = Object.freeze({
  getInitialURL,
  openURL,
  shareText: async () => {},
  addEventListener,
  removeEventListener,
});
