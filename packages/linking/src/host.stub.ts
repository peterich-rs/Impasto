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

export function addEventListener(
  type: string,
  listener: (event: { url: string }) => void,
): void {
  if (type === "url") {
    state.urlListener = listener;
  }
}

export function removeEventListener(
  type: string,
  listener: (event: { url: string }) => void,
): void {
  if (type === "url" && state.urlListener === listener) {
    state.urlListener = null;
  }
}
