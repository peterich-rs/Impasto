import type * as native from "@impasto/app-state/native";

export function getCurrentState(): string {
  return "active";
}

export const getRestorationState: typeof native.getRestorationState = () => null;
export const setRestorationState: typeof native.setRestorationState = () => {};

export const getWindowState: typeof native.getWindowState = () => null;
export const setWindowState: typeof native.setWindowState = () => {};

export const addEventListener: typeof native.addEventListener = () => {};
export const removeEventListener: typeof native.removeEventListener = () => {};
